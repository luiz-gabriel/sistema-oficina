//! Sessões de login de funcionário no servidor web, persistidas no SQLite.
//!
//! Validade ABSOLUTA: 10 horas a partir da autenticação (`VALIDADE_SEGUNDOS`). Uso, leitura ou reinício
//! nunca estendem o prazo. Guarda só o SHA-256 do token. Valores de tempo são inteiros (segundos Unix).
//! Quem chama passa o horário (`agora`), então os testes simulam o relógio sem esperar.
use chrono::{DateTime, Utc};
use rand::RngCore;
use rusqlite::{params, Connection, OptionalExtension};

use crate::error::AppResult;
use crate::seguranca::sha256_hex;

/// Validade absoluta de uma sessão de login: 10 horas.
pub const VALIDADE_SEGUNDOS: i64 = 10 * 60 * 60;

/// Sessão válida no momento consultado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessaoAtiva {
    pub usuario: String,
    pub dispositivo_id: i64,
    /// Fim absoluto da sessão (segundos Unix). Não muda com o uso.
    pub expira_em: i64,
}

/// Cria uma sessão e devolve o token (64 caracteres hexadecimais). O token não é guardado.
pub fn criar(conn: &Connection, usuario: &str, dispositivo_id: i64, agora: DateTime<Utc>) -> AppResult<String> {
    let mut b = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut b);
    let token = hex::encode(b);
    conn.execute(
        "INSERT INTO sessoes (token_hash, usuario, dispositivo_id, criado_em, expira_em) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![sha256_hex(&token), usuario, dispositivo_id, agora.to_rfc3339(), agora.timestamp() + VALIDADE_SEGUNDOS],
    )?;
    Ok(token)
}

/// Devolve a sessão se o token existe, não venceu e o dispositivo ainda está ATIVO.
/// Sessão vencida é apagada. Não renova o prazo.
pub fn validar(conn: &Connection, token: &str, agora: DateTime<Utc>) -> AppResult<Option<SessaoAtiva>> {
    if token.is_empty() {
        return Ok(None);
    }
    let hash = sha256_hex(token);
    let sessao: Option<SessaoAtiva> = conn
        .query_row(
            "SELECT s.usuario, s.dispositivo_id, s.expira_em FROM sessoes s
             JOIN dispositivos d ON d.id = s.dispositivo_id
             WHERE s.token_hash = ?1 AND d.situacao = 'ATIVO'",
            [&hash],
            |r| Ok(SessaoAtiva { usuario: r.get(0)?, dispositivo_id: r.get(1)?, expira_em: r.get(2)? }),
        )
        .optional()?;
    match sessao {
        Some(s) if s.expira_em > agora.timestamp() => Ok(Some(s)),
        Some(_) => {
            conn.execute("DELETE FROM sessoes WHERE token_hash = ?1", [&hash])?;
            Ok(None)
        }
        None => Ok(None),
    }
}

/// Segundos até o fim absoluto da sessão, nunca negativo (para o aviso dos 5 minutos).
pub fn segundos_restantes(sessao: &SessaoAtiva, agora: DateTime<Utc>) -> i64 {
    (sessao.expira_em - agora.timestamp()).max(0)
}

/// Encerra uma sessão imediatamente (botão Sair).
pub fn encerrar(conn: &Connection, token: &str) -> AppResult<()> {
    conn.execute("DELETE FROM sessoes WHERE token_hash = ?1", [sha256_hex(token)])?;
    Ok(())
}

/// Encerra todas as sessões de um dispositivo (revogação).
pub fn encerrar_dispositivo(conn: &Connection, dispositivo_id: i64) -> AppResult<()> {
    conn.execute("DELETE FROM sessoes WHERE dispositivo_id = ?1", [dispositivo_id])?;
    Ok(())
}

/// Encerra todas as sessões de um usuário (acesso retirado ou credencial alterada).
pub fn encerrar_usuario(conn: &Connection, usuario: &str) -> AppResult<()> {
    conn.execute("DELETE FROM sessoes WHERE usuario = ?1", [usuario])?;
    Ok(())
}
