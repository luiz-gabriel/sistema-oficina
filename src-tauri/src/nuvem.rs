//! Backup no Google Drive pela API REST (sem programa externo, estilo rclone).
//! Autorização: cliente OAuth do tipo "aplicativo para computador", criado pelo administrador no Google Cloud.
//! Fluxo com PKCE e redirecionamento para 127.0.0.1 em porta livre. Escopo `drive.file`: o app só enxerga
//! os arquivos que ele mesmo criou, então só apaga os próprios backups.
//! O client secret e o token de atualização ficam no banco, cifrados com chave derivada de `chave_rede`.
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use base64::Engine;
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use oficina_core::error::{AppError, AppResult, FieldError};
use oficina_core::registro;
use rand::RngCore;
use reqwest::blocking::{Client, Response};
use reqwest::Url;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::estado::Estado;

pub const PASTA_NUVEM: &str = "Oficina - Backups";
pub const MANTER_PADRAO: usize = 10;
const MANTER_MAXIMO: usize = 100;
const PREFIXO: &str = "oficina-";
const SUFIXO: &str = ".db";
const ESCOPO: &str = "https://www.googleapis.com/auth/drive.file";
const URL_AUTORIZACAO: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const URL_TOKEN: &str = "https://oauth2.googleapis.com/token";
const URL_REVOGAR: &str = "https://oauth2.googleapis.com/revoke";
const API: &str = "https://www.googleapis.com/drive/v3";
const API_UPLOAD: &str = "https://www.googleapis.com/upload/drive/v3/files";
const TEMPO_AUTORIZACAO: Duration = Duration::from_secs(300);

// Chaves na tabela config. Todas começam com "nuvem_" (a cópia enviada ao Drive não leva nenhuma delas).
const CHAVE_CLIENT_ID: &str = "nuvem_client_id";
const CHAVE_CLIENT_SEGREDO: &str = "nuvem_client_segredo";
const CHAVE_TOKEN: &str = "nuvem_token";
const CHAVE_EMAIL: &str = "nuvem_email";
const CHAVE_MANTER: &str = "nuvem_manter";

/// Estado da autorização em andamento (o navegador pode demorar; a tela consulta por `nuvem_status`).
#[derive(Clone, PartialEq)]
enum Conexao {
    Nenhuma,
    Aguardando,
    Falhou(String),
}

static CONEXAO: Mutex<Conexao> = Mutex::new(Conexao::Nenhuma);

// ---------------------------------------------------------------------------
// Lógica pura (testada sem rede)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct Arquivo {
    pub id: String,
    pub nome: String,
    pub criado_em: String,
}

/// Nome da cópia enviada: `oficina-AAAAMMDD-HHMMSS.db` (mesmo padrão das cópias locais).
pub fn nome_do_backup(momento: chrono::DateTime<chrono::Local>) -> String {
    format!("{PREFIXO}{}{SUFIXO}", momento.format("%Y%m%d-%H%M%S"))
}

/// Só aceita nomes do próprio sistema (sem barras nem caracteres estranhos).
pub fn e_backup_oficina(nome: &str) -> bool {
    nome.starts_with(PREFIXO)
        && nome.ends_with(SUFIXO)
        && nome.len() > PREFIXO.len() + SUFIXO.len()
        && nome.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
}

fn instante(texto: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(texto).map(|d| d.timestamp_millis()).unwrap_or(i64::MIN)
}

/// Quais arquivos apagar para sobrar só os `manter` mais recentes.
/// Ordena pela data de criação (mais nova primeiro) e, em empate, pelo nome. Nunca escolhe arquivo que não seja cópia do sistema.
pub fn ids_a_apagar(arquivos: &[Arquivo], manter: usize) -> Vec<String> {
    let mut validos: Vec<&Arquivo> = arquivos.iter().filter(|a| e_backup_oficina(&a.nome)).collect();
    validos.sort_by(|a, b| instante(&b.criado_em).cmp(&instante(&a.criado_em)).then_with(|| b.nome.cmp(&a.nome)));
    validos.into_iter().skip(manter).map(|a| a.id.clone()).collect()
}

pub fn validar_manter(valor: i64) -> AppResult<usize> {
    if (1..=MANTER_MAXIMO as i64).contains(&valor) {
        Ok(valor as usize)
    } else {
        Err(AppError::Validation(vec![FieldError {
            field: "manter".into(),
            message: format!("informe um número de 1 a {MANTER_MAXIMO}"),
        }]))
    }
}

fn escapar_busca(texto: &str) -> String {
    texto.replace('\\', "\\\\").replace('\'', "\\'")
}

pub fn busca_pasta() -> String {
    format!(
        "name = '{}' and mimeType = 'application/vnd.google-apps.folder' and trashed = false",
        escapar_busca(PASTA_NUVEM)
    )
}

pub fn busca_arquivos_da_pasta(pasta_id: &str) -> String {
    format!("'{}' in parents and trashed = false", escapar_busca(pasta_id))
}

/// Corpo `multipart/related` do upload (metadados em JSON + conteúdo do banco).
pub fn montar_upload(nome: &str, pasta_id: &str, conteudo: &[u8], fronteira: &str) -> Vec<u8> {
    let metadados = json!({ "name": nome, "parents": [pasta_id] });
    let mut corpo = Vec::with_capacity(conteudo.len() + 512);
    corpo.extend_from_slice(
        format!(
            "--{fronteira}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{metadados}\r\n\
             --{fronteira}\r\nContent-Type: application/octet-stream\r\n\r\n"
        )
        .as_bytes(),
    );
    corpo.extend_from_slice(conteudo);
    corpo.extend_from_slice(format!("\r\n--{fronteira}--\r\n").as_bytes());
    corpo
}

/// Desafio PKCE (S256): base64url sem preenchimento do SHA-256 do verificador.
pub fn desafio_pkce(verificador: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verificador.as_bytes()))
}

pub fn url_autorizacao(client_id: &str, redirecionamento: &str, estado: &str, desafio: &str) -> AppResult<String> {
    let url = Url::parse_with_params(
        URL_AUTORIZACAO,
        &[
            ("client_id", client_id),
            ("redirect_uri", redirecionamento),
            ("response_type", "code"),
            ("scope", ESCOPO),
            ("state", estado),
            ("code_challenge", desafio),
            ("code_challenge_method", "S256"),
            ("access_type", "offline"),
            ("prompt", "consent"),
        ],
    )
    .map_err(|e| AppError::Database(format!("endereço de autorização inválido: {e}")))?;
    Ok(url.to_string())
}

/// Parâmetros da primeira linha de uma requisição HTTP ("GET /?code=...&state=... HTTP/1.1").
pub fn parametros_da_requisicao(linha: &str) -> Vec<(String, String)> {
    let alvo = linha.split_whitespace().nth(1).unwrap_or("/");
    Url::parse(&format!("http://127.0.0.1{alvo}"))
        .map(|u| u.query_pairs().into_owned().collect())
        .unwrap_or_default()
}

fn hex_aleatorio(bytes: usize) -> String {
    let mut b = vec![0u8; bytes];
    rand::thread_rng().fill_bytes(&mut b);
    hex::encode(b)
}

fn base64_aleatorio(bytes: usize) -> String {
    let mut b = vec![0u8; bytes];
    rand::thread_rng().fill_bytes(&mut b);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b)
}

fn id_valido(id: &str) -> bool {
    !id.is_empty() && id.len() <= 200 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Caminho livre na pasta: se o nome já existe, acrescenta " (2)", " (3)"...
fn caminho_livre(dir: &Path, nome: &str) -> PathBuf {
    let mut caminho = dir.join(nome);
    let stem = nome.trim_end_matches(SUFIXO).to_string();
    let mut n = 2;
    while caminho.exists() {
        caminho = dir.join(format!("{stem}-{n}{SUFIXO}"));
        n += 1;
    }
    caminho
}

/// Chave de 32 bytes derivada da chave da rede (já guardada no banco e com alta entropia).
fn chave_cifra(chave_rede: &str) -> AppResult<[u8; 32]> {
    if chave_rede.is_empty() {
        return Err(AppError::Conflict("Ative o sistema antes de conectar ao Google Drive.".into()));
    }
    let bytes = hex::decode(chave_rede).map_err(|_| AppError::Database("chave da rede inválida".into()))?;
    let hk = hkdf::Hkdf::<Sha256>::new(None, &bytes);
    let mut out = [0u8; 32];
    hk.expand(b"oficina-nuvem-v1", &mut out).map_err(|_| AppError::Database("falha ao derivar chave".into()))?;
    Ok(out)
}

/// Cifra com ChaCha20-Poly1305 e nonce aleatório. Saída em hex: nonce (12 bytes) + texto cifrado.
pub fn selar(chave: &[u8; 32], texto: &[u8]) -> AppResult<String> {
    let cifra = ChaCha20Poly1305::new(Key::from_slice(chave));
    let mut nonce = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce);
    let cifrado = cifra
        .encrypt(Nonce::from_slice(&nonce), texto)
        .map_err(|_| AppError::Database("falha ao cifrar".into()))?;
    let mut tudo = nonce.to_vec();
    tudo.extend_from_slice(&cifrado);
    Ok(hex::encode(tudo))
}

pub fn abrir(chave: &[u8; 32], selado: &str) -> AppResult<Vec<u8>> {
    let erro = || AppError::Database("não foi possível ler os dados guardados da nuvem; conecte novamente".into());
    let bytes = hex::decode(selado).map_err(|_| erro())?;
    if bytes.len() < 12 {
        return Err(erro());
    }
    let (nonce, cifrado) = bytes.split_at(12);
    let cifra = ChaCha20Poly1305::new(Key::from_slice(chave));
    cifra.decrypt(Nonce::from_slice(nonce), cifrado).map_err(|_| erro())
}

// ---------------------------------------------------------------------------
// Configuração guardada no banco
// ---------------------------------------------------------------------------

/// Só o administrador configura e usa a nuvem (mesma exigência dos demais comandos de configuração, sem exceção).
fn administrador(est: &Estado) -> AppResult<String> {
    let sessao = est.sessao.lock().ok().and_then(|g| g.clone());
    match sessao {
        Some(s) if s.perfil == "administrador" => Ok(s.usuario),
        _ => Err(AppError::Conflict("Apenas o administrador pode usar o backup na nuvem.".into())),
    }
}

fn chave_do_banco(est: &Estado) -> AppResult<[u8; 32]> {
    let rede = est.get("chave_rede", "")?;
    chave_cifra(&rede)
}

fn ler_segredo(est: &Estado, chave: &str) -> AppResult<Option<String>> {
    let guardado = est.get(chave, "")?;
    if guardado.is_empty() {
        return Ok(None);
    }
    let bytes = abrir(&chave_do_banco(est)?, &guardado)?;
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| AppError::Database("dados da nuvem corrompidos; conecte novamente".into()))
}

fn conectado(est: &Estado) -> AppResult<bool> {
    Ok(!est.get(CHAVE_TOKEN, "")?.is_empty())
}

fn manter_configurado(est: &Estado) -> AppResult<usize> {
    let valor: i64 = est.get(CHAVE_MANTER, "")?.parse().unwrap_or(MANTER_PADRAO as i64);
    Ok(validar_manter(valor).unwrap_or(MANTER_PADRAO))
}

/// Grava no registro de atividades (sem segredos nem caminhos de token).
fn anotar(est: &Estado, usuario: &str, detalhe: &str) {
    let r = est
        .db()
        .and_then(|c| registro::registrar(&c, usuario, &est.computador, "backup no Google Drive", detalhe));
    if let Err(e) = r {
        eprintln!("aviso: não consegui registrar o backup na nuvem: {e}");
    }
}

pub fn configurar(est: &Estado, client_id: &str, client_segredo: &str, manter: i64) -> AppResult<()> {
    let usuario = administrador(est)?;
    let id = client_id.trim();
    let segredo = client_segredo.trim();
    let manter = validar_manter(manter)?;
    if id.is_empty() {
        return Err(AppError::Validation(vec![FieldError { field: "client_id".into(), message: "informe o Client ID".into() }]));
    }
    let anterior = est.get(CHAVE_CLIENT_ID, "")?;
    let trocou_projeto = anterior != id;
    if (trocou_projeto || est.get(CHAVE_CLIENT_SEGREDO, "")?.is_empty()) && segredo.is_empty() {
        return Err(AppError::Validation(vec![FieldError { field: "client_secret".into(), message: "informe o Client Secret".into() }]));
    }
    if trocou_projeto {
        // A autorização antiga pertence ao projeto antigo: precisa conectar de novo.
        est.definir(CHAVE_TOKEN, "")?;
        est.definir(CHAVE_EMAIL, "")?;
    }
    est.definir(CHAVE_CLIENT_ID, id)?;
    if !segredo.is_empty() {
        let selado = selar(&chave_do_banco(est)?, segredo.as_bytes())?;
        est.definir(CHAVE_CLIENT_SEGREDO, &selado)?;
    }
    est.definir(CHAVE_MANTER, &manter.to_string())?;
    anotar(est, &usuario, "salvou a configuração do backup na nuvem");
    Ok(())
}

pub fn status(est: &Estado) -> AppResult<Value> {
    administrador(est)?;
    let configurado = !est.get(CHAVE_CLIENT_ID, "")?.is_empty() && !est.get(CHAVE_CLIENT_SEGREDO, "")?.is_empty();
    let email = est.get(CHAVE_EMAIL, "")?;
    let (andamento, erro) = match CONEXAO.lock().map(|g| g.clone()).unwrap_or(Conexao::Nenhuma) {
        Conexao::Nenhuma => ("", None),
        Conexao::Aguardando => ("aguardando", None),
        Conexao::Falhou(msg) => ("erro", Some(msg)),
    };
    Ok(json!({
        "configurado": configurado,
        "client_id": est.get(CHAVE_CLIENT_ID, "")?,
        "conectado": conectado(est)?,
        "email": if email.is_empty() { Value::Null } else { json!(email) },
        "manter": manter_configurado(est)?,
        "padrao": MANTER_PADRAO,
        "pasta": PASTA_NUVEM,
        "conexao": andamento,
        "erro": erro,
    }))
}

// ---------------------------------------------------------------------------
// Rede (API do Google)
// ---------------------------------------------------------------------------

fn cliente() -> AppResult<Client> {
    Client::builder()
        .user_agent("oficina-backup/1.0")
        .timeout(Duration::from_secs(900))
        .build()
        .map_err(|e| AppError::Database(format!("não foi possível iniciar a conexão: {e}")))
}

fn rede(e: reqwest::Error) -> AppError {
    AppError::Offline(format!("Não consegui falar com o Google Drive. Verifique a internet. ({e})"))
}

fn verificar(resp: Response) -> AppResult<Response> {
    if resp.status().is_success() {
        return Ok(resp);
    }
    let status = resp.status().as_u16();
    let texto = resp.text().unwrap_or_default();
    let msg = serde_json::from_str::<Value>(&texto)
        .ok()
        .and_then(|v| v["error"]["message"].as_str().map(String::from))
        .unwrap_or_else(|| format!("código {status}"));
    Err(AppError::Conflict(format!("O Google Drive recusou o pedido: {msg}")))
}

fn texto_de(v: &Value, chave: &str) -> String {
    v[chave].as_str().unwrap_or_default().to_string()
}

#[derive(Deserialize)]
struct RespostaToken {
    access_token: Option<String>,
    refresh_token: Option<String>,
    error: Option<String>,
}

fn pedir_token(http: &Client, form: &[(&str, &str)]) -> AppResult<RespostaToken> {
    http.post(URL_TOKEN)
        .form(form)
        .send()
        .map_err(rede)?
        .json::<RespostaToken>()
        .map_err(|_| AppError::Offline("Resposta inesperada do Google ao pedir a autorização.".into()))
}

fn erro_de_autorizacao(motivo: Option<String>) -> AppError {
    match motivo.as_deref() {
        Some("invalid_grant") => AppError::Conflict(
            "A permissão com o Google não vale mais. Clique em \"Conectar ao Google Drive\" de novo.".into(),
        ),
        Some(outro) => AppError::Conflict(format!("O Google recusou a autorização ({outro}).")),
        None => AppError::Conflict("O Google recusou a autorização.".into()),
    }
}

/// Cliente HTTP e um token de acesso novo (trocado pelo token de atualização guardado).
fn acesso(est: &Estado) -> AppResult<(Client, String)> {
    let sem_conexao = || AppError::Conflict("Conecte ao Google Drive antes de usar o backup na nuvem.".into());
    let id = est.get(CHAVE_CLIENT_ID, "")?;
    let segredo = ler_segredo(est, CHAVE_CLIENT_SEGREDO)?.ok_or_else(sem_conexao)?;
    let renovacao = ler_segredo(est, CHAVE_TOKEN)?.ok_or_else(sem_conexao)?;
    let http = cliente()?;
    let resposta = pedir_token(
        &http,
        &[
            ("client_id", id.as_str()),
            ("client_secret", segredo.as_str()),
            ("refresh_token", renovacao.as_str()),
            ("grant_type", "refresh_token"),
        ],
    )?;
    match resposta.access_token {
        Some(token) => Ok((http, token)),
        None => Err(erro_de_autorizacao(resposta.error)),
    }
}

fn achar_pasta(http: &Client, token: &str) -> AppResult<Option<String>> {
    let resp = verificar(
        http.get(format!("{API}/files"))
            .bearer_auth(token)
            .query(&[("q", busca_pasta().as_str()), ("spaces", "drive"), ("fields", "files(id,name)"), ("pageSize", "1")])
            .send()
            .map_err(rede)?,
    )?;
    let v: Value = resp.json().map_err(|_| AppError::Offline("Resposta inesperada do Google Drive.".into()))?;
    Ok(v["files"][0]["id"].as_str().map(String::from))
}

fn pasta_de_backups(http: &Client, token: &str) -> AppResult<String> {
    if let Some(id) = achar_pasta(http, token)? {
        return Ok(id);
    }
    let resp = verificar(
        http.post(format!("{API}/files"))
            .bearer_auth(token)
            .json(&json!({ "name": PASTA_NUVEM, "mimeType": "application/vnd.google-apps.folder" }))
            .send()
            .map_err(rede)?,
    )?;
    let v: Value = resp.json().map_err(|_| AppError::Offline("Resposta inesperada do Google Drive.".into()))?;
    v["id"].as_str().map(String::from).ok_or_else(|| AppError::Offline("O Google Drive não devolveu a pasta criada.".into()))
}

fn arquivos_da_pasta(http: &Client, token: &str, pasta_id: &str) -> AppResult<Vec<Arquivo>> {
    let resp = verificar(
        http.get(format!("{API}/files"))
            .bearer_auth(token)
            .query(&[
                ("q", busca_arquivos_da_pasta(pasta_id).as_str()),
                ("fields", "files(id,name,createdTime)"),
                ("orderBy", "createdTime desc"),
                ("pageSize", "1000"),
            ])
            .send()
            .map_err(rede)?,
    )?;
    let v: Value = resp.json().map_err(|_| AppError::Offline("Resposta inesperada do Google Drive.".into()))?;
    let lista = v["files"].as_array().cloned().unwrap_or_default();
    Ok(lista
        .iter()
        .map(|f| Arquivo { id: texto_de(f, "id"), nome: texto_de(f, "name"), criado_em: texto_de(f, "createdTime") })
        .filter(|a| e_backup_oficina(&a.nome))
        .collect())
}

fn email_da_conta(http: &Client, token: &str) -> Option<String> {
    let resp = http
        .get(format!("{API}/about"))
        .bearer_auth(token)
        .query(&[("fields", "user(emailAddress)")])
        .send()
        .ok()?;
    let v: Value = resp.json().ok()?;
    v["user"]["emailAddress"].as_str().map(String::from)
}

/// Cópia do banco sem as chaves `nuvem_*` (nem o segredo nem o token vão para o Drive).
fn copia_sem_segredos(est: &Estado) -> AppResult<Vec<u8>> {
    let dir = est.dir.join(format!("nuvem-temp-{}", hex_aleatorio(6)));
    std::fs::create_dir_all(&dir).map_err(|e| AppError::Database(e.to_string()))?;
    let destino = dir.join("copia.db");
    let resultado = (|| -> AppResult<Vec<u8>> {
        {
            let c = est.db()?;
            c.execute("VACUUM INTO ?1", [destino.to_string_lossy().to_string()])?;
        }
        let copia = rusqlite::Connection::open(&destino)?;
        // Chaves de segurança não saem do computador: tokens do Drive e a chave da rede de sincronização.
        copia.execute("DELETE FROM config WHERE substr(chave, 1, 6) = 'nuvem_' OR chave IN ('chave_rede', 'chave_sessao', 'chave_ativacao')", [])?;
        copia.execute_batch("VACUUM")?;
        drop(copia);
        std::fs::read(&destino).map_err(|e| AppError::Database(e.to_string()))
    })();
    let _ = std::fs::remove_dir_all(&dir);
    resultado
}

fn fazer_envio(est: &Estado, manter: usize) -> AppResult<(String, usize)> {
    let nome = nome_do_backup(chrono::Local::now());
    let conteudo = copia_sem_segredos(est)?;
    let (http, token) = acesso(est)?;
    let pasta = pasta_de_backups(&http, &token)?;
    let fronteira = format!("oficina{}", hex_aleatorio(12));
    let corpo = montar_upload(&nome, &pasta, &conteudo, &fronteira);
    let enviado = verificar(
        http.post(format!("{API_UPLOAD}?uploadType=multipart&fields=id"))
            .bearer_auth(&token)
            .header("Content-Type", format!("multipart/related; boundary={fronteira}"))
            .body(corpo)
            .send()
            .map_err(rede)?,
    )?;
    let v: Value = enviado.json().map_err(|_| AppError::Offline("Resposta inesperada do Google Drive.".into()))?;
    if v["id"].as_str().is_none() {
        return Err(AppError::Offline("O Google Drive não confirmou o envio.".into()));
    }
    // Só depois do envio: apaga as cópias mais antigas que o programa criou nesta pasta.
    let arquivos = arquivos_da_pasta(&http, &token, &pasta)?;
    let mut apagados = 0;
    for id in ids_a_apagar(&arquivos, manter) {
        if id_valido(&id) {
            if let Ok(resp) = http.delete(format!("{API}/files/{id}")).bearer_auth(&token).send() {
                if resp.status().is_success() {
                    apagados += 1;
                }
            }
        }
    }
    Ok((nome, apagados))
}

fn enviar_interno(est: &Estado, usuario: &str) -> AppResult<Value> {
    if !conectado(est)? {
        return Err(AppError::Conflict("Conecte ao Google Drive antes de enviar o backup.".into()));
    }
    let manter = manter_configurado(est)?;
    match fazer_envio(est, manter) {
        Ok((nome, apagados)) => {
            anotar(est, usuario, &format!("enviou a cópia '{nome}'; cópias antigas removidas: {apagados}"));
            Ok(json!({ "nome": nome, "antigas_removidas": apagados }))
        }
        Err(e) => {
            anotar(est, usuario, &format!("erro ao enviar a cópia: {e}"));
            Err(e)
        }
    }
}

/// Botão "Enviar backup agora".
pub fn enviar_agora(est: &Estado) -> AppResult<Value> {
    let usuario = administrador(est)?;
    enviar_interno(est, &usuario)
}

/// Chamado pelo backup automático (a cada 6 horas). Só envia se a conta já estiver conectada.
pub fn enviar_automatico(est: &Estado) {
    if conectado(est).unwrap_or(false) {
        let _ = enviar_interno(est, "sistema");
    }
}

pub fn listar_agora(est: &Estado) -> AppResult<Value> {
    administrador(est)?;
    if !conectado(est)? {
        return Ok(json!([]));
    }
    let (http, token) = acesso(est)?;
    let Some(pasta) = achar_pasta(&http, &token)? else {
        return Ok(json!([]));
    };
    let arquivos = arquivos_da_pasta(&http, &token, &pasta)?;
    Ok(json!(arquivos
        .iter()
        .map(|a| json!({ "id": a.id, "nome": a.nome, "criado_em": a.criado_em }))
        .collect::<Vec<_>>()))
}

/// Baixa uma cópia para a pasta `backups-baixados` dos dados do app. NÃO substitui o banco: a restauração é manual.
pub fn baixar_agora(est: &Estado, id: &str) -> AppResult<String> {
    let usuario = administrador(est)?;
    if !id_valido(id) {
        return Err(AppError::Conflict("Arquivo inválido.".into()));
    }
    let (http, token) = acesso(est)?;
    let meta = verificar(
        http.get(format!("{API}/files/{id}"))
            .bearer_auth(&token)
            .query(&[("fields", "name")])
            .send()
            .map_err(rede)?,
    )?;
    let meta: Value = meta.json().map_err(|_| AppError::Offline("Resposta inesperada do Google Drive.".into()))?;
    let nome = texto_de(&meta, "name");
    if !e_backup_oficina(&nome) {
        return Err(AppError::Conflict("Este arquivo não é uma cópia do sistema.".into()));
    }
    let conteudo = verificar(
        http.get(format!("{API}/files/{id}"))
            .bearer_auth(&token)
            .query(&[("alt", "media")])
            .send()
            .map_err(rede)?,
    )?
    .bytes()
    .map_err(rede)?;
    let pasta = est.dir.join("backups-baixados");
    std::fs::create_dir_all(&pasta).map_err(|e| AppError::Database(e.to_string()))?;
    let destino = caminho_livre(&pasta, &nome);
    std::fs::write(&destino, &conteudo[..]).map_err(|e| AppError::Database(format!("não foi possível gravar: {e}")))?;
    anotar(est, &usuario, &format!("baixou a cópia '{nome}' do Google Drive"));
    Ok(destino.to_string_lossy().to_string())
}

pub fn desconectar_agora(est: &Estado) -> AppResult<()> {
    let usuario = administrador(est)?;
    let token = ler_segredo(est, CHAVE_TOKEN).ok().flatten();
    est.definir(CHAVE_TOKEN, "")?;
    est.definir(CHAVE_EMAIL, "")?;
    if let (Some(t), Ok(http)) = (token, cliente()) {
        // Revogação é best-effort: mesmo sem rede, a cópia local do token já foi apagada.
        let _ = http.post(URL_REVOGAR).form(&[("token", t.as_str())]).send();
    }
    if let Ok(mut c) = CONEXAO.lock() {
        *c = Conexao::Nenhuma;
    }
    anotar(est, &usuario, "desconectou a conta do Google Drive");
    Ok(())
}

// ---------------------------------------------------------------------------
// Autorização: abre o navegador e recebe o código no redirecionamento local
// ---------------------------------------------------------------------------

fn abrir_navegador(url: &str) -> std::io::Result<()> {
    #[cfg(target_os = "windows")]
    let processo = std::process::Command::new("rundll32").args(["url.dll,FileProtocolHandler", url]).spawn();
    #[cfg(target_os = "macos")]
    let processo = std::process::Command::new("open").arg(url).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let processo = std::process::Command::new("xdg-open").arg(url).spawn();
    processo.map(|_| ())
}

fn responder(conexao: &mut TcpStream, status: &str, corpo: &str) {
    let resposta = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{corpo}",
        corpo.len()
    );
    let _ = conexao.write_all(resposta.as_bytes());
}

const PAGINA_OK: &str = "<!doctype html><html lang=\"pt-BR\"><meta charset=\"utf-8\"><title>Oficina</title>\
<body style=\"font-family:sans-serif;padding:2rem\"><h2>Autorização recebida.</h2>\
<p>Pode fechar esta janela e voltar ao sistema da oficina.</p></body></html>";

/// Espera o Google redirecionar para 127.0.0.1 com o código. Ignora requisições sem código (ex.: favicon).
fn receber_codigo(listener: &TcpListener, estado_esperado: &str) -> AppResult<String> {
    listener
        .set_nonblocking(true)
        .map_err(|e| AppError::Database(format!("porta local indisponível: {e}")))?;
    let limite = Instant::now() + TEMPO_AUTORIZACAO;
    loop {
        if Instant::now() > limite {
            return Err(AppError::Conflict("O tempo para autorizar acabou. Clique em \"Conectar ao Google Drive\" de novo.".into()));
        }
        match listener.accept() {
            Ok((mut conexao, _)) => {
                let _ = conexao.set_nonblocking(false);
                let _ = conexao.set_read_timeout(Some(Duration::from_secs(5)));
                let mut buf = [0u8; 4096];
                let lidos = conexao.read(&mut buf).unwrap_or(0);
                let linha = String::from_utf8_lossy(&buf[..lidos]).lines().next().unwrap_or("").to_string();
                let params = parametros_da_requisicao(&linha);
                let pega = |k: &str| params.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
                if let Some(erro) = pega("error") {
                    responder(&mut conexao, "200 OK", "<p>Autorização não concedida. Pode fechar esta janela.</p>");
                    return Err(AppError::Conflict(format!("Autorização não concedida ({erro}).")));
                }
                let Some(codigo) = pega("code") else {
                    responder(&mut conexao, "404 Not Found", "");
                    continue;
                };
                if pega("state").as_deref() != Some(estado_esperado) {
                    responder(&mut conexao, "400 Bad Request", "<p>Resposta inválida.</p>");
                    return Err(AppError::Conflict("Resposta de autorização inválida. Tente conectar de novo.".into()));
                }
                responder(&mut conexao, "200 OK", PAGINA_OK);
                return Ok(codigo);
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => thread::sleep(Duration::from_millis(150)),
            Err(e) => return Err(AppError::Database(format!("falha na porta local: {e}"))),
        }
    }
}

/// Segunda metade da conexão, em segundo plano: espera o navegador, troca o código por token e guarda o token cifrado.
fn concluir_conexao(est: &Estado, listener: &TcpListener, estado: &str, verificador: &str, redirecionamento: &str) -> AppResult<Option<String>> {
    let codigo = receber_codigo(listener, estado)?;
    let id = est.get(CHAVE_CLIENT_ID, "")?;
    let segredo = ler_segredo(est, CHAVE_CLIENT_SEGREDO)?
        .ok_or_else(|| AppError::Conflict("Informe o Client Secret antes de conectar.".into()))?;
    let http = cliente()?;
    let resposta = pedir_token(
        &http,
        &[
            ("client_id", id.as_str()),
            ("client_secret", segredo.as_str()),
            ("code", codigo.as_str()),
            ("code_verifier", verificador),
            ("grant_type", "authorization_code"),
            ("redirect_uri", redirecionamento),
        ],
    )?;
    let renovacao = match resposta.refresh_token {
        Some(r) => r,
        None => return Err(erro_de_autorizacao(resposta.error)),
    };
    est.definir(CHAVE_TOKEN, &selar(&chave_do_banco(est)?, renovacao.as_bytes())?)?;
    // O e-mail só serve para mostrar "Conectado como ..."; se a consulta falhar, a conexão continua válida.
    let email = resposta.access_token.as_deref().and_then(|t| email_da_conta(&http, t));
    est.definir(CHAVE_EMAIL, email.as_deref().unwrap_or(""))?;
    Ok(email)
}

/// Botão "Conectar ao Google Drive". Abre o navegador e devolve o endereço (caso o navegador não abra sozinho).
/// A espera pelo Google acontece em segundo plano; a tela acompanha por `nuvem_status` (campo `conexao`).
pub fn conectar(est: &Arc<Estado>) -> AppResult<Value> {
    let usuario = administrador(est)?;
    let id = est.get(CHAVE_CLIENT_ID, "")?;
    if id.is_empty() || est.get(CHAVE_CLIENT_SEGREDO, "")?.is_empty() {
        return Err(AppError::Conflict("Salve o Client ID e o Client Secret antes de conectar.".into()));
    }
    {
        let em_andamento = CONEXAO.lock().map(|g| *g == Conexao::Aguardando).unwrap_or(false);
        if em_andamento {
            return Err(AppError::Conflict("Já existe uma autorização em andamento. Conclua no navegador ou aguarde alguns minutos.".into()));
        }
    }
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|e| AppError::Database(format!("não consegui abrir uma porta local: {e}")))?;
    let porta = listener.local_addr().map_err(|e| AppError::Database(e.to_string()))?.port();
    let redirecionamento = format!("http://127.0.0.1:{porta}/");
    let verificador = base64_aleatorio(32);
    let estado = hex_aleatorio(16);
    let url = url_autorizacao(&id, &redirecionamento, &estado, &desafio_pkce(&verificador))?;
    if let Ok(mut c) = CONEXAO.lock() {
        *c = Conexao::Aguardando;
    }
    let aberto = abrir_navegador(&url).is_ok();
    let est2 = est.clone();
    thread::spawn(move || {
        let resultado = concluir_conexao(&est2, &listener, &estado, &verificador, &redirecionamento);
        match resultado {
            Ok(email) => {
                if let Ok(mut c) = CONEXAO.lock() {
                    *c = Conexao::Nenhuma;
                }
                let _ = email; // o e-mail não entra no registro
                anotar(&est2, &usuario, "conectou a conta do Google Drive");
            }
            Err(e) => {
                if let Ok(mut c) = CONEXAO.lock() {
                    *c = Conexao::Falhou(e.to_string());
                }
                anotar(&est2, &usuario, &format!("erro ao conectar ao Google Drive: {e}"));
            }
        }
    });
    Ok(json!({ "url": url, "navegador_aberto": aberto }))
}

/// Executa uma operação de rede fora da thread principal, para a tela não travar.
pub async fn em_segundo_plano<T, F>(operacao: F) -> AppResult<T>
where
    F: FnOnce() -> AppResult<T> + Send + 'static,
    T: Send + 'static,
{
    tauri::async_runtime::spawn_blocking(operacao)
        .await
        .map_err(|e| AppError::Database(format!("operação interrompida: {e}")))?
}

#[cfg(test)]
mod testes {
    use super::*;
    use chrono::TimeZone;

    fn arq(id: &str, nome: &str, criado: &str) -> Arquivo {
        Arquivo { id: id.into(), nome: nome.into(), criado_em: criado.into() }
    }

    #[test]
    fn nome_do_backup_tem_data_e_hora() {
        let momento = chrono::Local.with_ymd_and_hms(2026, 11, 22, 14, 5, 9).unwrap();
        assert_eq!(nome_do_backup(momento), "oficina-20261122-140509.db");
    }

    #[test]
    fn so_aceita_nomes_do_sistema() {
        assert!(e_backup_oficina("oficina-20261122-140509.db"));
        assert!(!e_backup_oficina("../oficina-1.db"));
        assert!(!e_backup_oficina("outro.db"));
        assert!(!e_backup_oficina("oficina-.db"));
        assert!(!e_backup_oficina("oficina-1.pdf"));
    }

    #[test]
    fn mantem_os_mais_recentes_e_apaga_so_os_antigos_do_sistema() {
        let mut lista = Vec::new();
        for i in 0..12 {
            lista.push(arq(&format!("id{i}"), &format!("oficina-2026112{}-000000.db", i % 10),
                &format!("2026-11-{:02}T10:00:00.000Z", i + 1)));
        }
        // Arquivo que não é do sistema, mesmo antigo, nunca entra na lista de apagar.
        lista.push(arq("alheio", "planilha.xlsx", "2000-01-01T00:00:00Z"));
        let apagar = ids_a_apagar(&lista, 10);
        assert_eq!(apagar, vec!["id1".to_string(), "id0".to_string()]);
        assert!(ids_a_apagar(&lista, 12).is_empty());
        assert_eq!(ids_a_apagar(&lista, 0).len(), 12);
    }

    #[test]
    fn data_do_drive_manda_mais_que_o_nome() {
        let lista = vec![
            arq("novo", "oficina-20200101-000000.db", "2026-11-22T10:00:00Z"),
            arq("velho", "oficina-20991231-000000.db", "2026-01-01T10:00:00Z"),
        ];
        assert_eq!(ids_a_apagar(&lista, 1), vec!["velho".to_string()]);
    }

    #[test]
    fn manter_tem_limites() {
        assert_eq!(validar_manter(10).unwrap(), 10);
        assert!(validar_manter(0).is_err());
        assert!(validar_manter(101).is_err());
    }

    #[test]
    fn consulta_escapa_aspas() {
        assert!(busca_pasta().contains("name = 'Oficina - Backups'"));
        assert_eq!(escapar_busca("a'b\\c"), "a\\'b\\\\c");
        assert_eq!(busca_arquivos_da_pasta("abc"), "'abc' in parents and trashed = false");
    }

    #[test]
    fn corpo_do_upload_leva_nome_pasta_e_conteudo() {
        let corpo = montar_upload("oficina-1.db", "PASTA123", b"SQLite", "fronteira");
        let texto = String::from_utf8_lossy(&corpo).to_string();
        assert!(texto.starts_with("--fronteira\r\n"));
        assert!(texto.contains(r#""name":"oficina-1.db""#));
        assert!(texto.contains(r#""parents":["PASTA123"]"#));
        assert!(texto.contains("\r\n\r\nSQLite\r\n--fronteira--\r\n"));
    }

    #[test]
    fn pkce_confere_com_o_exemplo_da_rfc() {
        assert_eq!(
            desafio_pkce("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn url_de_autorizacao_pede_acesso_offline_e_pkce() {
        let url = url_autorizacao("cliente.apps", "http://127.0.0.1:5000/", "est", "des").unwrap();
        assert!(url.starts_with(URL_AUTORIZACAO));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("access_type=offline"));
        assert!(url.contains("scope=https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fdrive.file"));
        assert!(url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A5000%2F"));
    }

    #[test]
    fn le_codigo_do_redirecionamento() {
        let p = parametros_da_requisicao("GET /?code=abc&state=xyz HTTP/1.1");
        assert!(p.contains(&("code".to_string(), "abc".to_string())));
        assert!(p.contains(&("state".to_string(), "xyz".to_string())));
    }

    #[test]
    fn cifra_ida_e_volta_e_recusa_chave_errada() {
        let chave = [9u8; 32];
        let selado = selar(&chave, b"token-secreto").unwrap();
        assert!(!selado.contains("token-secreto"));
        assert_eq!(abrir(&chave, &selado).unwrap(), b"token-secreto");
        assert!(abrir(&[1u8; 32], &selado).is_err());
    }

    #[test]
    fn chave_de_cifra_exige_rede_ativada() {
        assert!(chave_cifra("").is_err());
        assert!(chave_cifra(&"ab".repeat(32)).is_ok());
    }

}
