//! Servidor web local: serve a interface (`ui/`) para navegadores da rede local e expõe rotas HTTP
//! explícitas (ver `rotas.rs`). Não usa a rede entre computadores (`rede.rs`): é outra porta e outro protocolo.
//! Transporte HTTP feito pela biblioteca `tiny_http` (HTTP/1.1, uma requisição por conexão).
//! Escuta na interface da rede local (0.0.0.0). Sem HTTPS nesta fase: o cookie não recebe `Secure`.
pub mod porta;
mod rotas;

use std::collections::HashMap;
use std::io::Read;
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};

use crate::estado::Estado;

/// Nome do cookie de sessão do navegador.
pub const NOME_COOKIE: &str = "oficina_sessao";
const LIMITE_CORPO: usize = 64 * 1024;

/// Estado do servidor web guardado em `Estado`.
#[derive(Default)]
pub struct Web {
    /// Sessões de navegador: token do cookie -> usuário. Só em memória.
    pub sessoes: Mutex<HashMap<String, rotas::SessaoWeb>>,
    ativo: Mutex<Option<Ativo>>,
    erro: Mutex<Option<String>>,
}

struct Ativo {
    endereco: Endereco,
    rodando: Arc<AtomicBool>,
}

/// Endereço efetivo que o painel e o QR Code devem mostrar.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Endereco {
    pub ip: Option<String>,
    pub porta: u16,
    pub url: String,
    pub origem: &'static str,
    /// Porta salva da execução anterior, se ela mudou agora (links antigos podem deixar de funcionar).
    pub porta_anterior: Option<u16>,
    pub aviso: Option<String>,
}

/// Pedido já lido. Só o necessário para as rotas.
pub struct Requisicao {
    pub metodo: String,
    pub caminho: String,
    pub cookie: Option<String>,
    pub corpo: Vec<u8>,
}

/// Resposta a ser escrita de volta.
#[derive(Debug)]
pub struct Resposta {
    pub status: u16,
    pub tipo: String,
    pub cabecalhos: Vec<(String, String)>,
    pub corpo: Vec<u8>,
}

impl Resposta {
    pub fn json(status: u16, valor: &Value) -> Resposta {
        Resposta {
            status,
            tipo: "application/json; charset=utf-8".into(),
            cabecalhos: Vec::new(),
            corpo: valor.to_string().into_bytes(),
        }
    }

    /// Erro no mesmo formato do restante do sistema: `{ok: false, erro: {kind, message}}`.
    pub fn erro(status: u16, kind: &str, mensagem: &str) -> Resposta {
        Resposta::json(status, &json!({"ok": false, "erro": {"kind": kind, "message": mensagem}}))
    }

}

impl Web {
    /// Situação para a tela de configuração: ativo, endereço ou erro.
    pub fn situacao(&self) -> Value {
        let ativo = self.ativo.lock().unwrap_or_else(|e| e.into_inner());
        json!({
            "ativo": ativo.is_some(),
            "endereco": ativo.as_ref().map(|a| a.endereco.clone()),
            "erro": self.erro.lock().unwrap_or_else(|e| e.into_inner()).clone(),
        })
    }

    fn definir_erro(&self, erro: Option<String>) {
        *self.erro.lock().unwrap_or_else(|e| e.into_inner()) = erro;
    }
}

/// Liga o servidor web se ainda não estiver ligado e devolve o endereço efetivo.
/// Erro (com mensagem para o usuário) quando não há como servir: não indica que está disponível.
pub fn iniciar(est: &Arc<Estado>, ui: PathBuf) -> Result<Endereco, String> {
    {
        let ativo = est.web.ativo.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(a) = ativo.as_ref() {
            return Ok(a.endereco.clone());
        }
    }
    let resultado = subir(est, ui);
    match &resultado {
        Ok(_) => est.web.definir_erro(None),
        Err(e) => {
            eprintln!("aviso: servidor web não iniciado: {e}");
            est.web.definir_erro(Some(e.clone()));
        }
    }
    resultado
}

fn subir(est: &Arc<Estado>, ui: PathBuf) -> Result<Endereco, String> {
    if porta::rodando_como_root() {
        return Err("O servidor web não sobe com permissão de administrador (root). Abra o sistema com um usuário comum.".into());
    }
    let salva = porta::ler_salva(&est.dir);
    let escolhida = porta::escolher(salva, porta::ligar_lan)
        .map_err(|e| format!("Não há porta disponível para o servidor web: {e}"))?;
    // Só grava depois do bind: a porta salva é sempre uma que este computador conseguiu abrir.
    if let Err(e) = porta::gravar(&est.dir, escolhida.porta) {
        eprintln!("aviso: não consegui salvar a porta do servidor web: {e}");
    }
    let ip = crate::rede::meu_ip();
    let porta_final = escolhida.porta;
    let url = match ip {
        Some(ip) if porta_final == 80 => format!("http://{ip}/"),
        Some(ip) => format!("http://{ip}:{porta_final}/"),
        None => format!("http://127.0.0.1:{porta_final}/"),
    };
    let porta_anterior = salva.filter(|p| *p != porta_final);
    let mut avisos = Vec::new();
    if let Some(anterior) = porta_anterior {
        avisos.push(format!("A porta mudou de {anterior} para {porta_final}. Links antigos podem deixar de funcionar."));
    }
    if ip.is_none() {
        avisos.push("Não encontrei o endereço da rede local: só este computador acessa o sistema por enquanto.".into());
    }
    let endereco = Endereco {
        ip: ip.map(|i: Ipv4Addr| i.to_string()),
        porta: porta_final,
        url,
        origem: escolhida.origem.nome(),
        porta_anterior,
        aviso: if avisos.is_empty() { None } else { Some(avisos.join(" ")) },
    };
    let rodando = Arc::new(AtomicBool::new(true));
    let vigia = rodando.clone();
    let (est2, ui2, listener) = (est.clone(), ui, escolhida.listener);
    thread::spawn(move || servir(listener, est2, ui2, vigia));
    *est.web.ativo.lock().unwrap_or_else(|e| e.into_inner()) = Some(Ativo { endereco: endereco.clone(), rodando });
    Ok(endereco)
}

/// Desliga o servidor web (ex.: o computador saiu dos modos de rede). Não faz nada se não estiver ligado.
pub fn parar(est: &Estado) {
    let ativo = est.web.ativo.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(a) = ativo {
        a.rodando.store(false, Ordering::SeqCst);
        // Acorda a thread que espera conexões; ela confere a flag e sai.
        let _ = TcpStream::connect_timeout(
            &std::net::SocketAddr::from(([127, 0, 0, 1], a.endereco.porta)),
            Duration::from_millis(300),
        );
    }
    est.web.definir_erro(None);
}

fn servir(listener: TcpListener, est: Arc<Estado>, ui: PathBuf, rodando: Arc<AtomicBool>) {
    let servidor = match tiny_http::Server::from_listener(listener, None) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("erro ao iniciar o servidor web: {e}");
            return;
        }
    };
    for pedido in servidor.incoming_requests() {
        if !rodando.load(Ordering::SeqCst) {
            break;
        }
        let (est, ui) = (est.clone(), ui.clone());
        thread::spawn(move || atender(pedido, &est, &ui));
    }
}

/// Atende uma requisição: converte para `Requisicao`, trata e responde.
pub fn atender(pedido: tiny_http::Request, est: &Estado, ui: &Path) {
    let mut pedido = pedido;
    let resposta = match ler_requisicao(&mut pedido) {
        Ok(req) => tratar(est, ui, &req),
        Err(r) => r,
    };
    let _ = pedido.respond(para_tiny(&resposta));
}

/// Lê método, caminho, cookie e corpo com limites. Erros viram resposta 400/413 pronta.
pub fn ler_requisicao(pedido: &mut tiny_http::Request) -> Result<Requisicao, Resposta> {
    let ruim = || Resposta::erro(400, "requisicao", "Requisição inválida.");
    if pedido.body_length().unwrap_or(0) > LIMITE_CORPO {
        return Err(Resposta::erro(413, "tamanho", "Pedido grande demais."));
    }
    let mut corpo = Vec::new();
    pedido
        .as_reader()
        .take(LIMITE_CORPO as u64 + 1)
        .read_to_end(&mut corpo)
        .map_err(|_| ruim())?;
    if corpo.len() > LIMITE_CORPO {
        return Err(Resposta::erro(413, "tamanho", "Pedido grande demais."));
    }
    let url = pedido.url().to_string();
    let caminho = url.split('?').next().unwrap_or("/").to_string();
    let cookie = pedido
        .headers()
        .iter()
        .find(|h| h.field.equiv("Cookie"))
        .map(|h| h.value.as_str().to_string());
    Ok(Requisicao {
        metodo: pedido.method().as_str().to_string(),
        caminho,
        cookie,
        corpo,
    })
}

/// Converte a resposta interna para a da biblioteca, com cabeçalhos de segurança.
fn para_tiny(r: &Resposta) -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    let mut resp = tiny_http::Response::from_data(r.corpo.clone())
        .with_status_code(r.status)
        .with_header(header("Content-Type", &r.tipo))
        .with_header(header("X-Content-Type-Options", "nosniff"))
        .with_header(header("X-Frame-Options", "DENY"))
        .with_header(header("Referrer-Policy", "no-referrer"))
        .with_header(header("Cache-Control", "no-store"))
        .with_header(header("Connection", "close"));
    for (k, v) in &r.cabecalhos {
        resp = resp.with_header(header(k, v));
    }
    resp
}

fn header(nome: &str, valor: &str) -> tiny_http::Header {
    tiny_http::Header::from_bytes(nome.as_bytes(), valor.as_bytes()).expect("cabeçalho válido")
}

pub fn tratar(est: &Estado, ui: &Path, req: &Requisicao) -> Resposta {
    if req.caminho == "/api" || req.caminho.starts_with("/api/") {
        return rotas::tratar(est, req);
    }
    if req.metodo != "GET" {
        return Resposta::erro(405, "requisicao", "Método não permitido.");
    }
    arquivo_estatico(ui, &req.caminho)
}

/// Serve um arquivo de `ui/`. Recusa qualquer caminho que saia da pasta.
fn arquivo_estatico(ui: &Path, caminho: &str) -> Resposta {
    let nao_achou = || Resposta::erro(404, "not_found", "Página não encontrada.");
    let relativo = match caminho.trim_start_matches('/') {
        "" => "index.html",
        r => r,
    };
    if relativo.contains('\\') || relativo.contains('\0') || relativo.split('/').any(|p| p == "..") {
        return nao_achou();
    }
    let Ok(base) = ui.canonicalize() else { return nao_achou() };
    let Ok(arq) = base.join(relativo).canonicalize() else { return nao_achou() };
    if !arq.starts_with(&base) || !arq.is_file() {
        return nao_achou();
    }
    match std::fs::read(&arq) {
        Ok(bytes) => Resposta { status: 200, tipo: tipo_de(&arq), cabecalhos: Vec::new(), corpo: bytes },
        Err(_) => nao_achou(),
    }
}

fn tipo_de(p: &Path) -> String {
    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let t = match ext.as_str() {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    };
    t.into()
}

/// Caminho da pasta `ui/` quando o projeto está no disco (desenvolvimento).
pub fn pasta_ui() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("ui")
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn estatico_nao_sai_da_pasta_ui() {
        let base = std::env::temp_dir().join(format!("oficina_ui_teste_{}", std::process::id()));
        std::fs::create_dir_all(base.join("js")).unwrap();
        std::fs::write(base.join("index.html"), "<!doctype html>").unwrap();
        std::fs::write(base.join("js").join("a.js"), "export {}").unwrap();
        std::fs::write(base.parent().unwrap().join("segredo_teste.txt"), "x").unwrap();
        assert_eq!(arquivo_estatico(&base, "/").status, 200);
        assert_eq!(arquivo_estatico(&base, "/js/a.js").tipo, "text/javascript; charset=utf-8");
        assert_eq!(arquivo_estatico(&base, "/inexistente.html").status, 404);
        assert_eq!(arquivo_estatico(&base, "/../segredo_teste.txt").status, 404);
        assert_eq!(arquivo_estatico(&base, "/%2e%2e/segredo_teste.txt").status, 404);
        let _ = std::fs::remove_file(base.parent().unwrap().join("segredo_teste.txt"));
        let _ = std::fs::remove_dir_all(&base);
    }
}
