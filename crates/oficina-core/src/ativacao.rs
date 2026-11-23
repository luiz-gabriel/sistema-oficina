//! Ativação: valida o código (simulação do servidor de ativação) e registra a instalação.
//! A chave nunca é guardada em texto: guarda-se o hash e a chave da rede derivada.
use rusqlite::Connection;

use chrono::Utc;

use crate::config;
use crate::hora;
use crate::error::{AppError, AppResult};
use crate::seguranca::{chave_rede, normalizar_codigo, sha256_hex};

/// Códigos do arquivo `ativacao_codigos.md` (raiz do projeto), embutidos no binário.
const ARQUIVO_CODIGOS: &str = include_str!("../../../ativacao_codigos.md");

pub fn codigos_validos() -> Vec<String> {
    let mut out = Vec::new();
    for linha in ARQUIVO_CODIGOS.lines() {
        for token in linha.split(|c: char| c == '`' || c == ' ' || c == '|') {
            let t = token.trim();
            if t.len() == 19 && t.chars().enumerate().all(|(i, c)| {
                if i % 5 == 4 { c == '-' } else { c.is_ascii_uppercase() || c.is_ascii_digit() }
            }) {
                out.push(t.to_string());
            }
        }
    }
    out
}

pub fn esta_ativado(conn: &Connection) -> AppResult<bool> {
    Ok(config::obter(conn, "ativado_em")?.is_some())
}

/// Ativa a instalação usando só o relógio do computador (sem consulta de rede).
pub fn ativar(conn: &Connection, codigo: &str) -> AppResult<()> {
    ativar_com(conn, codigo, || hora::escolher(Vec::new(), Utc::now)).map(|_| ())
}

/// Ativa a instalação. `obter_hora` só é chamada depois de validar o código e confirmar que não está ativado.
/// Grava: data de ativação (`ativado_em`), fonte (`ativacao_fonte`: "ntp" ou "relogio_local"),
/// desvio do relógio local em relação à hora atômica (`ativacao_desvio_segundos`, se houver),
/// identificador próprio e chave da rede.
pub fn ativar_com(conn: &Connection, codigo: &str, obter_hora: impl FnOnce() -> hora::Hora) -> AppResult<hora::Hora> {
    if esta_ativado(conn)? {
        return Err(AppError::Conflict("Este computador já está ativado.".into()));
    }
    let codigo = normalizar_codigo(codigo);
    if !codigos_validos().contains(&codigo) {
        return Err(AppError::Validation(vec![crate::error::FieldError {
            field: "codigo".into(),
            message: "Código de ativação inválido.".into(),
        }]));
    }
    let hora = obter_hora();
    config::definir(conn, "ativacao_hash", &sha256_hex(&codigo))?;
    config::definir(conn, "chave_rede", &hex::encode(chave_rede(&codigo)))?;
    config::definir(conn, "dispositivo_id", &uuid::Uuid::new_v4().to_string())?;
    config::definir(conn, "ativado_em", &hora.instante.to_rfc3339())?;
    config::definir(conn, "ativacao_fonte", hora.fonte)?;
    if let Some(desvio) = hora.desvio_segundos {
        config::definir(conn, "ativacao_desvio_segundos", &format!("{desvio:.3}"))?;
    }
    Ok(hora)
}
