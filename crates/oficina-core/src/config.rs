//! Configurações locais do computador (modo de funcionamento, identificação, pendências).
use rusqlite::{params, Connection, OptionalExtension};

use crate::error::AppResult;

pub fn obter(conn: &Connection, chave: &str) -> AppResult<Option<String>> {
    Ok(conn
        .query_row("SELECT valor FROM config WHERE chave = ?1", [chave], |r| r.get::<_, String>(0))
        .optional()?)
}

pub fn definir(conn: &Connection, chave: &str, valor: &str) -> AppResult<()> {
    conn.execute(
        "INSERT INTO config (chave, valor) VALUES (?1, ?2) ON CONFLICT(chave) DO UPDATE SET valor = excluded.valor",
        params![chave, valor],
    )?;
    Ok(())
}

pub fn obter_ou(conn: &Connection, chave: &str, padrao: &str) -> AppResult<String> {
    Ok(obter(conn, chave)?.unwrap_or_else(|| padrao.to_string()))
}
