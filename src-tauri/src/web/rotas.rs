//! Rotas `/api/...` do navegador. Cada rota é explícita: nada de repassar comandos por nome.
//! As regras continuam em oficina-core (`comandos::executar`, `funcionarios::entrar`); aqui só há HTTP.
//!
//! Ponto único de autenticação: `exigir_sessao_com_tempo`. Confere o token no SQLite (validade absoluta
//! de 10 horas, aparelho ATIVO) e se o funcionário ainda tem acesso.
//! Login: com cabeçalho `X-Oficina-Dispositivo` (aparelho pareado) ou, sem ele, só pelo próprio
//! computador principal (conexão de loopback).
use chrono::Utc;
use oficina_core::comandos::{self, Contexto, ADMINISTRADOR};
use oficina_core::dispositivos;
use oficina_core::error::AppError;
use oficina_core::{ativacao, funcionarios, registro, sessoes};
use serde::Deserialize;
use serde_json::{json, Value};

use super::{Requisicao, Resposta, NOME_COOKIE};
use crate::estado::Estado;

/// Leituras expostas pela web. Cada uma chama um comando de leitura de oficina-core.
struct Leitura {
    caminho: &'static str,
    comando: &'static str,
    /// Se true, aceita `/caminho/<id>` e passa `id` ao comando.
    com_id: bool,
}

const LEITURAS: &[Leitura] = &[
    Leitura { caminho: "/api/clientes", comando: "clientes_listar", com_id: false },
    Leitura { caminho: "/api/veiculos", comando: "veiculos_listar", com_id: false },
    Leitura { caminho: "/api/itens", comando: "itens_listar", com_id: false },
    Leitura { caminho: "/api/ordens", comando: "ordens_listar", com_id: false },
    Leitura { caminho: "/api/ordens/", comando: "ordens_buscar", com_id: true },
    Leitura { caminho: "/api/fornecedores", comando: "fornecedores_listar", com_id: false },
    Leitura { caminho: "/api/funcionarios", comando: "funcionarios_listar", com_id: false },
    Leitura { caminho: "/api/caixa/situacao", comando: "caixa_situacao", com_id: false },
];

#[derive(Deserialize)]
struct Login {
    #[serde(default)]
    usuario: String,
    #[serde(default)]
    senha: String,
}

#[derive(Deserialize)]
struct EnvioPedido {
    #[serde(default)]
    codigo: String,
    #[serde(default)]
    nome: String,
}

#[derive(Deserialize)]
struct ConsultaPedido {
    #[serde(default)]
    pedido_id: i64,
    #[serde(default)]
    segredo: String,
}

pub fn tratar(est: &Estado, req: &Requisicao) -> Resposta {
    let caminho = req.caminho.as_str();
    let metodo = req.metodo.as_str();
    match (metodo, caminho) {
        ("GET", "/api/estado") => return estado(est, req),
        ("POST", "/api/login") => return login(est, req),
        ("POST", "/api/logout") => return logout(est, req),
        (_, "/api/estado") | (_, "/api/login") | (_, "/api/logout") => return metodo_errado(),
        _ => {}
    }
    if caminho.starts_with("/api/dispositivos") || caminho.starts_with("/api/pareamento/") {
        return dispositivos_rota(est, req);
    }
    for rota in LEITURAS {
        let id = if rota.com_id {
            caminho.strip_prefix(rota.caminho).and_then(|r| r.parse::<i64>().ok())
        } else if caminho == rota.caminho {
            Some(0)
        } else {
            None
        };
        let Some(id) = id else { continue };
        if metodo != "GET" {
            return metodo_errado();
        }
        return leitura(est, req, rota, id);
    }
    Resposta::erro(404, "not_found", "Rota não encontrada.")
}

fn metodo_errado() -> Resposta {
    Resposta::erro(405, "not_found", "Método não permitido nesta rota.")
}

/// Única porta de entrada para rotas que exigem login. Confere o cookie no SQLite (validade absoluta,
/// aparelho ATIVO) e se o usuário ainda tem acesso. Devolve também os segundos restantes da sessão.
fn exigir_sessao_com_tempo(est: &Estado, req: &Requisicao) -> Result<(Contexto, i64), Resposta> {
    let nao_logado = || Resposta::erro(401, "conflict", "Faça login para continuar.");
    let token = req.cookie.as_deref().and_then(|c| cookie(c, NOME_COOKIE)).ok_or_else(nao_logado)?;
    let agora = Utc::now();
    let c = est.db().map_err(erro_app)?;
    let sessao = sessoes::validar(&c, &token, agora).map_err(erro_app)?.ok_or_else(nao_logado)?;
    // Perfil do cadastro atual: se o acesso foi revogado, a sessão deixa de valer.
    let perfil: Option<String> = c
        .query_row(
            "SELECT perfil FROM funcionarios WHERE login = ?1 AND senha_hash IS NOT NULL",
            [&sessao.usuario],
            |r| r.get(0),
        )
        .ok();
    match perfil {
        Some(perfil) => {
            dispositivos::registrar_acesso(&c, sessao.dispositivo_id, agora).map_err(erro_app)?;
            let restante = sessoes::segundos_restantes(&sessao, agora);
            Ok((Contexto { usuario: sessao.usuario, perfil, computador: est.computador.clone() }, restante))
        }
        None => {
            sessoes::encerrar(&c, &token).map_err(erro_app)?;
            Err(nao_logado())
        }
    }
}

fn exigir_sessao(est: &Estado, req: &Requisicao) -> Result<Contexto, Resposta> {
    exigir_sessao_com_tempo(est, req).map(|(c, _)| c)
}

/// Só o administrador passa. Usado nas rotas de dispositivos (a checagem também fica no oficina-core).
fn exigir_administrador(est: &Estado, req: &Requisicao) -> Result<Contexto, Resposta> {
    let ctx = exigir_sessao(est, req)?;
    if ctx.perfil != ADMINISTRADOR {
        return Err(Resposta::erro(403, "conflict", "Só o administrador pode gerenciar dispositivos."));
    }
    Ok(ctx)
}

fn leitura(est: &Estado, req: &Requisicao, rota: &Leitura, id: i64) -> Resposta {
    let ctx = match exigir_sessao(est, req) {
        Ok(c) => c,
        Err(r) => return r,
    };
    let args = if rota.com_id { json!({"id": id}) } else { json!({}) };
    let resultado = est.db().map_err(erro_app).and_then(|c| {
        comandos::executar(&c, rota.comando, &args, &ctx).map_err(erro_app)
    });
    match resultado {
        Ok(valor) => Resposta::json(200, &valor),
        Err(r) => r,
    }
}

fn estado(est: &Estado, req: &Requisicao) -> Resposta {
    let (logado, restante) = match exigir_sessao_com_tempo(est, req) {
        Ok((c, restante)) => (Some(c.usuario), Some(restante)),
        Err(_) => (None, None),
    };
    // `modo()` também pega o banco: lê antes de travar a conexão aqui, senão trava.
    let modo = est.modo();
    let dados = (|| -> Result<Value, AppError> {
        let c = est.db()?;
        let ativado = ativacao::esta_ativado(&c)?;
        let tem_admin = funcionarios::existe_administrador(&c)?;
        Ok(json!({
            "ativado": ativado,
            "tem_admin": tem_admin,
            "modo": modo,
            "logado": logado,
            "sessao_restante_segundos": restante,
        }))
    })();
    match dados {
        Ok(v) => Resposta::json(200, &v),
        Err(e) => erro_app(e),
    }
}

fn login(est: &Estado, req: &Requisicao) -> Resposta {
    let Ok(dados) = serde_json::from_slice::<Login>(&req.corpo) else {
        return Resposta::erro(400, "validation", "Informe usuário e senha.");
    };
    let agora = Utc::now();
    let dispositivo = match dispositivo_do_login(est, req) {
        Ok(d) => d,
        Err(r) => return r,
    };
    let entrada = match est.db().and_then(|c| funcionarios::entrar(&c, &dados.usuario, &dados.senha, agora)) {
        Ok(e) => e,
        Err(e) => return erro_app(e),
    };
    match entrada {
        funcionarios::Entrada::Ok(f) => {
            let usuario = f.login.clone().unwrap_or_default();
            let token = match est.db().and_then(|c| sessoes::criar(&c, &usuario, dispositivo, agora)) {
                Ok(t) => t,
                Err(e) => return erro_app(e),
            };
            anotar(est, &usuario, "entrou pelo navegador");
            let mut resp = Resposta::json(
                200,
                &json!({"usuario": usuario, "nome": f.nome, "perfil": f.perfil, "sessao_segundos": sessoes::VALIDADE_SEGUNDOS}),
            );
            resp.cabecalhos.push(("Set-Cookie".into(), cookie_de(&token, false)));
            resp
        }
        funcionarios::Entrada::Falhou => Resposta::erro(401, "conflict", "Login ou senha incorretos."),
        funcionarios::Entrada::Bloqueado { .. } => Resposta::erro(
            423,
            "conflict",
            "Acesso bloqueado por excesso de tentativas erradas. Tente de novo em 5 minutos.",
        ),
    }
}

/// Aparelho que está fazendo o login. Com credencial (cabeçalho), o token precisa ser de um aparelho ATIVO.
/// Sem credencial, só o computador principal, e só por conexão local. Outro computador da rede é recusado.
fn dispositivo_do_login(est: &Estado, req: &Requisicao) -> Result<i64, Resposta> {
    let agora = Utc::now();
    if let Some(token) = req.dispositivo.as_deref() {
        let id = est.db().and_then(|c| dispositivos::autenticar_dispositivo(&c, token, agora)).map_err(erro_app)?;
        return id.ok_or_else(|| Resposta::erro(401, "conflict", "Este aparelho não está autorizado. Faça o pareamento de novo."));
    }
    if !req.local {
        return Err(Resposta::erro(401, "conflict", "Este aparelho precisa ser pareado antes de entrar."));
    }
    let c = est.db().map_err(erro_app)?;
    let id = dispositivos::garantir_principal(&c, &est.computador, agora).map_err(erro_app)?;
    if !dispositivos::esta_ativo(&c, id).map_err(erro_app)? {
        return Err(Resposta::erro(403, "conflict", "O computador principal foi revogado. Entre por um aparelho autorizado."));
    }
    Ok(id)
}

fn logout(est: &Estado, req: &Requisicao) -> Resposta {
    let token = req.cookie.as_deref().and_then(|c| cookie(c, NOME_COOKIE));
    if let Some(t) = &token {
        // Encerra já no banco. Se a sessão ainda existia, registra quem saiu.
        let usuario = est
            .db()
            .ok()
            .and_then(|c| sessoes::validar(&c, t, Utc::now()).ok().flatten())
            .map(|s| s.usuario);
        let _ = est.db().and_then(|c| sessoes::encerrar(&c, t));
        if let Some(usuario) = usuario {
            anotar(est, &usuario, "saiu pelo navegador");
        }
    }
    let mut resp = Resposta::json(200, &json!({"ok": true}));
    resp.cabecalhos.push(("Set-Cookie".into(), cookie_de("", true)));
    resp
}

/// Rotas de dispositivos e pareamento. As duas de pareamento são públicas (o aparelho ainda não tem credencial):
/// só aceitam código válido ou segredo do pedido. As demais exigem administrador.
fn dispositivos_rota(est: &Estado, req: &Requisicao) -> Resposta {
    let metodo = req.metodo.as_str();
    let partes: Vec<&str> = req.caminho.trim_start_matches("/api/").split('/').collect();
    match (metodo, partes.as_slice()) {
        ("POST", ["pareamento", "pedidos"]) => return pareamento_enviar(est, req),
        ("POST", ["pareamento", "consultar"]) => return pareamento_consultar(est, req),
        _ => {}
    }
    let ctx = match exigir_administrador(est, req) {
        Ok(c) => c,
        Err(r) => return r,
    };
    match (metodo, partes.as_slice()) {
        ("GET", ["dispositivos"]) => executar_admin(est, &ctx, "dispositivos_listar", json!({})),
        ("GET", ["dispositivos", "pedidos"]) => executar_admin(est, &ctx, "pareamento_listar_pedidos", json!({})),
        ("POST", ["dispositivos", "pareamento"]) => iniciar_pareamento(est, &ctx),
        ("POST", ["dispositivos", "pedidos", id, "aprovar"]) => com_id(est, &ctx, "pareamento_aprovar", id),
        ("POST", ["dispositivos", "pedidos", id, "recusar"]) => com_id(est, &ctx, "pareamento_recusar", id),
        ("POST", ["dispositivos", id, "revogar"]) => com_id(est, &ctx, "dispositivos_revogar", id),
        (_, ["dispositivos"]) | (_, ["dispositivos", "pareamento"]) | (_, ["dispositivos", "pedidos"]) => metodo_errado(),
        _ => Resposta::erro(404, "not_found", "Rota não encontrada."),
    }
}

fn com_id(est: &Estado, ctx: &Contexto, cmd: &str, id: &str) -> Resposta {
    match id.parse::<i64>() {
        Ok(id) => executar_admin(est, ctx, cmd, json!({"id": id})),
        Err(_) => Resposta::erro(404, "not_found", "Rota não encontrada."),
    }
}

fn executar_admin(est: &Estado, ctx: &Contexto, cmd: &str, args: Value) -> Resposta {
    let r = est.db().map_err(erro_app).and_then(|c| comandos::executar(&c, cmd, &args, ctx).map_err(erro_app));
    match r {
        Ok(valor) => Resposta::json(200, &valor),
        Err(r) => r,
    }
}

/// Gera o código para o administrador exibir no principal, junto com o endereço do servidor.
fn iniciar_pareamento(est: &Estado, ctx: &Contexto) -> Resposta {
    let r = est.db().map_err(erro_app).and_then(|c| comandos::executar(&c, "dispositivos_iniciar_pareamento", &json!({}), ctx).map_err(erro_app));
    let mut valor = match r {
        Ok(v) => v,
        Err(r) => return r,
    };
    valor["endereco"] = match est.web.endereco() {
        Some(e) => json!({"ip": e.ip, "porta": e.porta, "url": e.url}),
        None => Value::Null,
    };
    Resposta::json(200, &valor)
}

fn pareamento_enviar(est: &Estado, req: &Requisicao) -> Resposta {
    let Ok(dados) = serde_json::from_slice::<EnvioPedido>(&req.corpo) else {
        return Resposta::erro(400, "validation", "Informe o código e o nome do aparelho.");
    };
    let r = est.db().and_then(|c| dispositivos::enviar_pedido(&c, &dados.codigo, &dados.nome, Utc::now()));
    match r {
        Ok(p) => Resposta::json(200, &json!({"pedido_id": p.pedido_id, "segredo": p.segredo})),
        Err(e) => erro_app(e),
    }
}

/// O aparelho acompanha o pedido. O token sai só uma vez, na entrega.
fn pareamento_consultar(est: &Estado, req: &Requisicao) -> Resposta {
    let Ok(dados) = serde_json::from_slice::<ConsultaPedido>(&req.corpo) else {
        return Resposta::erro(400, "validation", "Informe o pedido e o segredo.");
    };
    let r = est.db().and_then(|c| dispositivos::consultar_pedido(&c, dados.pedido_id, &dados.segredo, Utc::now()));
    match r {
        Ok(dispositivos::Consulta::Pendente) => Resposta::json(200, &json!({"situacao": "PENDENTE"})),
        Ok(dispositivos::Consulta::Recusado) => Resposta::json(200, &json!({"situacao": "RECUSADO"})),
        Ok(dispositivos::Consulta::Liberado { token }) => Resposta::json(200, &json!({"situacao": "LIBERADO", "token": token})),
        Ok(dispositivos::Consulta::JaEntregue) => Resposta::json(200, &json!({"situacao": "JA_ENTREGUE"})),
        Err(e) => erro_app(e),
    }
}

/// Cookie de sessão: HttpOnly e SameSite=Strict. `Secure` entra junto com HTTPS (fase própria).
fn cookie_de(token: &str, apagar: bool) -> String {
    let validade = if apagar { "; Max-Age=0" } else { "" };
    format!("{NOME_COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict{validade}")
}

fn cookie(cabecalho: &str, nome: &str) -> Option<String> {
    cabecalho.split(';').find_map(|par| {
        let (k, v) = par.trim().split_once('=')?;
        (k == nome && !v.is_empty()).then(|| v.to_string())
    })
}

fn anotar(est: &Estado, usuario: &str, acao: &str) {
    let r = est.db().and_then(|c| registro::registrar(&c, usuario, &est.computador, acao, "{}"));
    if let Err(e) = r {
        eprintln!("aviso: não consegui registrar a atividade '{acao}': {e}");
    }
}

/// Erro de domínio no mesmo formato que o restante do sistema usa.
fn erro_app(e: AppError) -> Resposta {
    let status = match &e {
        AppError::Validation(_) => 422,
        AppError::NotFound(_) => 404,
        AppError::Conflict(_) => 409,
        AppError::Offline(_) => 503,
        AppError::Database(_) => 500,
    };
    Resposta::json(status, &json!({"ok": false, "erro": serde_json::to_value(&e).unwrap_or(Value::Null)}))
}

#[cfg(test)]
mod testes {
    use super::*;
    use oficina_core::{db, funcionarios::{AcessoInput, FuncionarioInput}};
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::Arc;

    fn estado_de_teste() -> Arc<Estado> {
        let dir = std::env::temp_dir().join(format!("oficina_web_teste_{}_{}", std::process::id(), rand_id()));
        let conn = db::open_in_memory().expect("banco em memória");
        {
            let f = funcionarios::criar(&conn, &FuncionarioInput { nome: Some("Ana".into()), cargo: Some("Gerente".into()), telefone: None }).unwrap();
            funcionarios::dar_acesso(&conn, f.id, &AcessoInput {
                login: Some("ana".into()),
                senha: Some("segredo123".into()),
                senha_hash: None,
                perfil: Some("administrador".into()),
            })
            .unwrap();
        }
        Arc::new(Estado::novo(conn, dir, 0))
    }

    fn rand_id() -> u32 {
        use rand::RngCore;
        rand::thread_rng().next_u32()
    }

    /// Sobe o servidor de verdade numa porta livre e faz uma requisição HTTP crua.
    fn pedir(est: &Arc<Estado>, pedido: &str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let porta = listener.local_addr().unwrap().port();
        let ui = std::env::temp_dir().join("oficina_ui_inexistente_teste");
        let servidor = super::super::iniciar_http(listener, est.clone(), ui);
        // Pede o fechamento da conexão, para o cliente de teste ler até o fim da resposta.
        let pedido = pedido.replacen("\r\n", "\r\nConnection: close\r\n", 1);
        let mut s = TcpStream::connect(("127.0.0.1", porta)).unwrap();
        s.write_all(pedido.as_bytes()).unwrap();
        let mut resp = String::new();
        s.read_to_string(&mut resp).unwrap();
        servidor.shutdown();
        resp
    }

    fn status(resp: &str) -> u16 {
        resp.split_whitespace().nth(1).unwrap().parse().unwrap()
    }

    fn corpo(resp: &str) -> &str {
        resp.split("\r\n\r\n").nth(1).unwrap_or("")
    }

    fn get(caminho: &str, cookie: Option<&str>) -> String {
        let c = cookie.map(|c| format!("Cookie: {c}\r\n")).unwrap_or_default();
        format!("GET {caminho} HTTP/1.1\r\nHost: t\r\n{c}\r\n")
    }

    fn login_pedido(senha: &str) -> String {
        let corpo = format!(r#"{{"usuario":"ana","senha":"{senha}"}}"#);
        format!("POST /api/login HTTP/1.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{corpo}", corpo.len())
    }

    fn token_do_cookie(resp: &str) -> String {
        // Nomes de cabeçalho não diferenciam maiúsculas; o hyper os envia em minúsculas.
        let linha = resp
            .lines()
            .find(|l| l.to_ascii_lowercase().starts_with("set-cookie:"))
            .expect("cookie de sessão");
        let valor = linha.split_once(':').unwrap().1.trim();
        valor.split(';').next().unwrap().to_string()
    }

    #[test]
    fn leitura_sem_login_recebe_401() {
        let est = estado_de_teste();
        assert_eq!(status(&pedir(&est, &get("/api/clientes", None))), 401);
    }

    #[test]
    fn rota_inexistente_recebe_404() {
        let est = estado_de_teste();
        assert_eq!(status(&pedir(&est, &get("/api/nao_existe", None))), 404);
        assert_eq!(status(&pedir(&est, &get("/api/ordens/abc", None))), 404);
    }

    #[test]
    fn login_errado_nao_cria_sessao() {
        let est = estado_de_teste();
        let r = pedir(&est, &login_pedido("errada"));
        assert_eq!(status(&r), 401);
        assert!(!r.contains("Set-Cookie"));
        let n: i64 = est.db().unwrap().query_row("SELECT COUNT(*) FROM sessoes", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn login_certo_emite_cookie_e_libera_leitura() {
        let est = estado_de_teste();
        let r = pedir(&est, &login_pedido("segredo123"));
        assert_eq!(status(&r), 200);
        assert!(r.contains("HttpOnly"), "cookie deve ser HttpOnly: {r}");
        assert!(r.contains("SameSite=Strict"));
        let cookie = token_do_cookie(&r);
        let lista = pedir(&est, &get("/api/clientes", Some(&cookie)));
        assert_eq!(status(&lista), 200, "{lista}");
        assert_eq!(corpo(&lista).trim(), "[]");
    }

    #[test]
    fn logout_invalida_o_cookie() {
        let est = estado_de_teste();
        let cookie = token_do_cookie(&pedir(&est, &login_pedido("segredo123")));
        let saida = format!("POST /api/logout HTTP/1.1\r\nCookie: {cookie}\r\nContent-Length: 0\r\n\r\n");
        assert_eq!(status(&pedir(&est, &saida)), 200);
        assert_eq!(status(&pedir(&est, &get("/api/clientes", Some(&cookie)))), 401);
    }

    #[test]
    fn cookie_de_usuario_removido_deixa_de_valer() {
        let est = estado_de_teste();
        let cookie = token_do_cookie(&pedir(&est, &login_pedido("segredo123")));
        est.db().unwrap().execute("UPDATE funcionarios SET senha_hash = NULL", []).unwrap();
        assert_eq!(status(&pedir(&est, &get("/api/clientes", Some(&cookie)))), 401);
    }

    #[test]
    fn https_responde_com_a_identidade_da_instalacao() {
        let est = estado_de_teste();
        let id = super::super::tls::identidade(&est.dir, vec!["localhost".into()]).unwrap();
        let cert = reqwest::Certificate::from_pem(&id.cert_pem).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let porta = listener.local_addr().unwrap().port();
        let ui = std::env::temp_dir().join("oficina_ui_inexistente_teste");
        let servidor = super::super::iniciar_https(listener, est.clone(), ui, id);
        let cliente = reqwest::blocking::Client::builder()
            .add_root_certificate(cert)
            .resolve("localhost", std::net::SocketAddr::from(([127, 0, 0, 1], porta)))
            .build()
            .unwrap();
        let resp = cliente.get(format!("https://localhost:{porta}/api/estado")).send().unwrap();
        assert_eq!(resp.status(), 200);
        // Certificado que não é o da instalação deve ser recusado pelo cliente.
        let outro = reqwest::Certificate::from_pem(&super::super::tls::identidade(&std::env::temp_dir().join(format!("oficina_outra_{}", std::process::id())), vec!["localhost".into()]).unwrap().cert_pem).unwrap();
        let cliente_outro = reqwest::blocking::Client::builder()
            .tls_certs_only(vec![outro])
            .resolve("localhost", std::net::SocketAddr::from(([127, 0, 0, 1], porta)))
            .build()
            .unwrap();
        assert!(cliente_outro.get(format!("https://localhost:{porta}/api/estado")).send().is_err());
        servidor.shutdown();
    }

    #[test]
    fn estado_publico_responde_sem_login() {
        let est = estado_de_teste();
        let r = pedir(&est, &get("/api/estado", None));
        assert_eq!(status(&r), 200);
        assert!(corpo(&r).contains("\"logado\":null"), "{r}");
    }

    fn post(caminho: &str, cookie: Option<&str>, corpo: &str) -> String {
        let c = cookie.map(|c| format!("Cookie: {c}\r\n")).unwrap_or_default();
        format!("POST {caminho} HTTP/1.1\r\nHost: t\r\nContent-Type: application/json\r\n{c}Content-Length: {}\r\n\r\n{corpo}", corpo.len())
    }

    fn json_do(resp: &str) -> Value {
        serde_json::from_str(corpo(resp)).unwrap_or(Value::Null)
    }

    fn requisicao(metodo: &str, caminho: &str, dispositivo: Option<&str>, local: bool, corpo: &str) -> Requisicao {
        Requisicao {
            metodo: metodo.into(),
            caminho: caminho.into(),
            cookie: None,
            dispositivo: dispositivo.map(str::to_string),
            local,
            corpo: corpo.as_bytes().to_vec(),
        }
    }

    #[test]
    fn outro_computador_sem_pareamento_nao_entra() {
        let est = estado_de_teste();
        let r = tratar(&est, &requisicao("POST", "/api/login", None, false, r#"{"usuario":"ana","senha":"segredo123"}"#));
        assert_eq!(r.status, 401);
        assert!(!r.cabecalhos.iter().any(|(k, _)| k == "Set-Cookie"));
    }

    #[test]
    fn fluxo_de_pareamento_e_login_do_aparelho() {
        let est = estado_de_teste();
        let admin = token_do_cookie(&pedir(&est, &login_pedido("segredo123")));
        // Administrador no principal gera o código.
        let cod = pedir(&est, &post("/api/dispositivos/pareamento", Some(&admin), ""));
        assert_eq!(status(&cod), 200, "{cod}");
        let codigo = json_do(&cod)["codigo"].as_str().unwrap().to_string();
        assert_eq!(codigo.len(), 6);
        assert_eq!(json_do(&cod)["endereco"], Value::Null, "sem servidor ligado o endereço fica vazio");
        // Aparelho envia o código (rota pública).
        let envio = pedir(&est, &post("/api/pareamento/pedidos", None, &format!(r#"{{"codigo":"{codigo}","nome":"Celular da Ana"}}"#)));
        assert_eq!(status(&envio), 200, "{envio}");
        let pedido = json_do(&envio);
        let (pedido_id, segredo) = (pedido["pedido_id"].as_i64().unwrap(), pedido["segredo"].as_str().unwrap().to_string());
        // Enquanto não aprovado, o aparelho só vê PENDENTE.
        let consulta = format!(r#"{{"pedido_id":{pedido_id},"segredo":"{segredo}"}}"#);
        assert_eq!(json_do(&pedir(&est, &post("/api/pareamento/consultar", None, &consulta)))["situacao"], "PENDENTE");
        // Administrador vê o pedido e aprova.
        let pendentes = json_do(&pedir(&est, &get("/api/dispositivos/pedidos", Some(&admin))));
        assert_eq!(pendentes[0]["nome_aparelho"], "Celular da Ana");
        let aprovado = pedir(&est, &post(&format!("/api/dispositivos/pedidos/{pedido_id}/aprovar"), Some(&admin), ""));
        assert_eq!(status(&aprovado), 200, "{aprovado}");
        // Aparelho recebe o token uma vez.
        let liberado = json_do(&pedir(&est, &post("/api/pareamento/consultar", None, &consulta)));
        assert_eq!(liberado["situacao"], "LIBERADO");
        let token = liberado["token"].as_str().unwrap().to_string();
        assert_eq!(json_do(&pedir(&est, &post("/api/pareamento/consultar", None, &consulta)))["situacao"], "JA_ENTREGUE");
        // Login do aparelho: vindo de outro computador, com o token certo, entra.
        let entrou = tratar(&est, &requisicao("POST", "/api/login", Some(&token), false, r#"{"usuario":"ana","senha":"segredo123"}"#));
        assert_eq!(entrou.status, 200);
        assert!(entrou.cabecalhos.iter().any(|(k, _)| k == "Set-Cookie"));
        // Token errado não entra.
        let errado = tratar(&est, &requisicao("POST", "/api/login", Some("00"), false, r#"{"usuario":"ana","senha":"segredo123"}"#));
        assert_eq!(errado.status, 401);
        // Lista de dispositivos mostra o aparelho e nunca o token ou o hash.
        let lista = pedir(&est, &get("/api/dispositivos", Some(&admin)));
        assert!(lista.contains("Celular da Ana"));
        assert!(!lista.contains(&token));
        assert!(!lista.contains("credencial"));
    }

    #[test]
    fn nao_administrador_nao_ve_nem_gera_pareamento() {
        let est = estado_de_teste();
        {
            let c = est.db().unwrap();
            let f = funcionarios::criar(&c, &FuncionarioInput { nome: Some("Bia".into()), cargo: None, telefone: None }).unwrap();
            funcionarios::dar_acesso(&c, f.id, &AcessoInput {
                login: Some("bia".into()),
                senha: Some("outrasenha1".into()),
                senha_hash: None,
                perfil: Some("funcionario".into()),
            })
            .unwrap();
        }
        let corpo = r#"{"usuario":"bia","senha":"outrasenha1"}"#;
        let cookie = token_do_cookie(&pedir(&est, &format!("POST /api/login HTTP/1.1\r\nHost: t\r\nContent-Length: {}\r\n\r\n{corpo}", corpo.len())));
        assert_eq!(status(&pedir(&est, &get("/api/dispositivos", Some(&cookie)))), 403);
        assert_eq!(status(&pedir(&est, &post("/api/dispositivos/pareamento", Some(&cookie), ""))), 403);
        assert_eq!(status(&pedir(&est, &get("/api/dispositivos", None))), 401);
    }

    #[test]
    fn cookie_so_le_o_nome_pedido() {
        assert_eq!(cookie("a=1; oficina_sessao=xyz; b=2", NOME_COOKIE).as_deref(), Some("xyz"));
        assert_eq!(cookie("a=1", NOME_COOKIE), None);
    }

}
