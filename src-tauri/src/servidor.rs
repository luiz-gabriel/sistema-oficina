//! Lado servidor: atende snapshots e comandos enviados pelos outros computadores da licença.
//! Snapshot e comandos exigem token de sessão de usuário obtido com `login`, emitido para o mesmo dispositivo.
//! Dispositivo autorizado sem usuário logado não recebe dados. A identidade (usuário e
//! dispositivo) vem do token; o perfil vem do cadastro atual do servidor. O cliente não escolhe quem é.
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Instant;

use oficina_core::comandos::Contexto;
use oficina_core::error::AppError;
use oficina_core::funcionarios::{self, Entrada};
use oficina_core::sync;
use serde_json::{json, Value};

use crate::estado::Estado;
use crate::rede::{self, Info};

pub fn erro_app(e: AppError) -> Value {
    json!({"ok": false, "erro": serde_json::to_value(&e).unwrap_or(Value::Null)})
}

fn fora_do_servidor() -> Value {
    json!({"ok": false, "erro": {"kind": "offline", "message": "Este computador não está atuando como servidor."}})
}

pub fn tratar_conexao(est: Arc<Estado>, s: TcpStream) {
    let Ok(Some(id)) = est.identidade() else { return };
    let papel = est.papel();
    let Ok(mut canal) = rede::aceitar(s, &id, &papel) else { return };
    rede::lembrar(&est, &canal.info.dispositivo, &canal.info.computador, &canal.info.ativado_em);
    est.ja_visto(&canal.info.dispositivo);
    while let Ok(msg) = canal.receber() {
        let resp = tratar(&est, &canal.info, &msg);
        if canal.enviar(&resp).is_err() {
            break;
        }
        est.ja_visto(&canal.info.dispositivo);
    }
}

fn tratar(est: &Estado, conexao: &Info, msg: &Value) -> Value {
    match msg["t"].as_str() {
        Some("login") => tratar_login(est, conexao, msg),
        Some("logout") => {
            if let Some(token) = msg["token"].as_str() {
                est.rede.sessoes.encerrar(token);
            }
            json!({"ok": true})
        }
        Some("snapshot") => {
            if est.papel() != "servidor" {
                return fora_do_servidor();
            }
            let token = msg["token"].as_str().unwrap_or("");
            if let Err(e) = sync::exigir_sessao(&est.rede.sessoes, token, &conexao.dispositivo, Instant::now()) {
                let mut resp = erro_app(e);
                resp["sessao_invalida"] = json!(true);
                return resp;
            }
            match est.db().and_then(|c| sync::snapshot(&c)) {
                Ok(d) => json!({"ok": true, "dados": d}),
                Err(e) => erro_app(e),
            }
        }
        Some("cmd") => tratar_comando(est, conexao, msg),
        _ => json!({"ok": false, "erro": {"kind": "database", "message": "mensagem desconhecida"}}),
    }
}

/// Confere usuário e senha no cadastro do servidor (com o limite de tentativas) e emite o token.
fn tratar_login(est: &Estado, conexao: &Info, msg: &Value) -> Value {
    if est.papel() != "servidor" {
        return fora_do_servidor();
    }
    let usuario = msg["usuario"].as_str().unwrap_or("");
    let senha = msg["senha"].as_str().unwrap_or("");
    let entrada = match est.db().and_then(|c| funcionarios::entrar(&c, usuario, senha, chrono::Utc::now())) {
        Ok(e) => e,
        Err(e) => return erro_app(e),
    };
    match entrada {
        Entrada::Ok(f) => {
            let login = f.login.clone().unwrap_or_default();
            let token = est.rede.sessoes.emitir(&login, &f.perfil, &conexao.dispositivo, Instant::now());
            json!({"ok": true, "token": token, "usuario": login, "nome": f.nome, "perfil": f.perfil})
        }
        Entrada::Falhou => erro_app(AppError::Conflict("Login ou senha incorretos.".into())),
        Entrada::Bloqueado { .. } => erro_app(AppError::Conflict(
            "Acesso bloqueado por excesso de tentativas erradas. Tente de novo em 5 minutos.".into(),
        )),
    }
}

fn tratar_comando(est: &Estado, conexao: &Info, msg: &Value) -> Value {
    if est.papel() != "servidor" {
        return fora_do_servidor();
    }
    let token = msg["token"].as_str().unwrap_or("");
    // Usuário e dispositivo vêm da sessão emitida no login. O que o cliente informa em `usuario` é ignorado.
    let Some(sessao) = est.rede.sessoes.validar(token, &conexao.dispositivo, Instant::now()) else {
        // `sessao_invalida`: o cliente não marca a alteração como erro; ela continua pendente lá até um novo login.
        let mut resp = erro_app(AppError::Conflict(crate::sincronia::MSG_SESSAO_EXPIRADA.into()));
        resp["sessao_invalida"] = json!(true);
        return resp;
    };
    let nome = msg["nome"].as_str().unwrap_or("").to_string();
    let args = msg["args"].clone();
    let usuario = sessao.usuario.clone();
    let computador = conexao.computador.clone();
    let c = match est.db() {
        Ok(c) => c,
        Err(e) => return erro_app(e),
    };
    // Perfil do cadastro atual: se o usuário foi removido, a sessão deixa de valer.
    let perfil: Option<String> = c
        .query_row("SELECT perfil FROM funcionarios WHERE login = ?1", [&usuario], |r| r.get(0))
        .ok();
    let Some(perfil) = perfil else {
        return erro_app(AppError::Conflict("Usuário não cadastrado no servidor.".into()));
    };
    let ctx = Contexto { usuario: usuario.clone(), perfil, computador: computador.clone() };
    // Com o identificador da alteração, um reenvio não é aplicado de novo (ver `sync::processar_escrita`).
    let escrita = sync::Escrita { uuid: msg["uuid"].as_str(), nome: &nome, args: &args, base: msg["base"].as_str(), ctx: &ctx };
    match sync::processar_escrita(&c, &escrita) {
        Ok(sync::Resposta::Aplicada(valor)) => json!({"ok": true, "valor": valor}),
        Ok(sync::Resposta::Repetida(valor)) => json!({"ok": true, "valor": valor, "repetida": true}),
        Ok(sync::Resposta::Conflito(atual)) => json!({"ok": false, "conflito": atual}),
        Err(e) => erro_app(e),
    }
}
