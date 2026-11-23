//! Rotas `/api/...` do navegador. Cada rota é explícita: nada de repassar comandos por nome.
//! As regras continuam em oficina-core (`comandos::executar`, `funcionarios::entrar`); aqui só há HTTP.
//!
//! Ponto único de autenticação: `exigir_sessao`. Quando entrar pareamento, limite de dispositivos
//! e sessões de 10 horas (próximas fases), é nele que a checagem de expiração e dispositivo deve entrar.
use oficina_core::comandos::{self, Contexto};
use oficina_core::error::AppError;
use oficina_core::{ativacao, funcionarios, registro, seguranca};
use serde::Deserialize;
use serde_json::{json, Value};

use super::{Requisicao, Resposta, NOME_COOKIE};
use crate::estado::Estado;

/// Quem está logado no navegador. Guardado só em memória.
#[derive(Clone, Debug)]
pub struct SessaoWeb {
    pub usuario: String,
}

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

/// Única porta de entrada para rotas que exigem login. Confere o cookie e se o usuário ainda tem acesso.
fn exigir_sessao(est: &Estado, req: &Requisicao) -> Result<Contexto, Resposta> {
    let nao_logado = || Resposta::erro(401, "conflict", "Faça login para continuar.");
    let token = req.cookie.as_deref().and_then(|c| cookie(c, NOME_COOKIE)).ok_or_else(nao_logado)?;
    let sessao = est
        .web
        .sessoes
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&token)
        .cloned()
        .ok_or_else(nao_logado)?;
    // Perfil do cadastro atual: se o acesso foi revogado, a sessão deixa de valer.
    let c = est.db().map_err(erro_app)?;
    let perfil: Option<String> = c
        .query_row(
            "SELECT perfil FROM funcionarios WHERE login = ?1 AND senha_hash IS NOT NULL",
            [&sessao.usuario],
            |r| r.get(0),
        )
        .ok();
    match perfil {
        Some(perfil) => Ok(Contexto { usuario: sessao.usuario, perfil, computador: est.computador.clone() }),
        None => {
            est.web.sessoes.lock().unwrap_or_else(|e| e.into_inner()).remove(&token);
            Err(nao_logado())
        }
    }
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
    let logado = exigir_sessao(est, req).ok().map(|c| c.usuario);
    // `modo()` também pega o banco: lê antes de travar a conexão aqui, senão trava.
    let modo = est.modo();
    let dados = (|| -> Result<Value, AppError> {
        let c = est.db()?;
        let ativado = ativacao::esta_ativado(&c)?;
        let tem_admin = funcionarios::existe_administrador(&c)?;
        Ok(json!({"ativado": ativado, "tem_admin": tem_admin, "modo": modo, "logado": logado}))
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
    let agora = chrono::Utc::now();
    let entrada = match est.db().and_then(|c| funcionarios::entrar(&c, &dados.usuario, &dados.senha, agora)) {
        Ok(e) => e,
        Err(e) => return erro_app(e),
    };
    match entrada {
        funcionarios::Entrada::Ok(f) => {
            let usuario = f.login.clone().unwrap_or_default();
            let token = format!("{}{}", hex::encode(seguranca::nonce_aleatorio()), hex::encode(seguranca::nonce_aleatorio()));
            est.web
                .sessoes
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(token.clone(), SessaoWeb { usuario: usuario.clone() });
            anotar(est, &usuario, "entrou pelo navegador");
            let mut resp = Resposta::json(200, &json!({"usuario": usuario, "nome": f.nome, "perfil": f.perfil}));
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

fn logout(est: &Estado, req: &Requisicao) -> Resposta {
    let token = req.cookie.as_deref().and_then(|c| cookie(c, NOME_COOKIE));
    if let Some(t) = &token {
        let removida = est.web.sessoes.lock().unwrap_or_else(|e| e.into_inner()).remove(t);
        if let Some(s) = removida {
            anotar(est, &s.usuario, "saiu pelo navegador");
        }
    }
    let mut resp = Resposta::json(200, &json!({"ok": true}));
    resp.cabecalhos.push(("Set-Cookie".into(), cookie_de("", true)));
    resp
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
        assert!(est.web.sessoes.lock().unwrap().is_empty());
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

    #[test]
    fn cookie_so_le_o_nome_pedido() {
        assert_eq!(cookie("a=1; oficina_sessao=xyz; b=2", NOME_COOKIE).as_deref(), Some("xyz"));
        assert_eq!(cookie("a=1", NOME_COOKIE), None);
    }

}
