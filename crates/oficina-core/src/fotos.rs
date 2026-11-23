//! Fotos do carro (anexadas ao veículo). Ficam no banco, então entram no backup e na sincronização.
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use chrono::Utc;
use rusqlite::{params, Connection, Row};
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult, FieldError};

pub const TAMANHO_MAXIMO: usize = 5 * 1024 * 1024;
const TIPOS: &[&str] = &["image/jpeg", "image/png", "image/webp"];

#[derive(Debug, Clone, Serialize)]
pub struct Foto {
    pub id: i64,
    pub veiculo_id: i64,
    pub nome: String,
    pub mime: String,
    pub criado_em: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct FotoInput {
    pub veiculo_id: Option<i64>,
    pub nome: Option<String>,
    pub mime: Option<String>,
    /// Conteúdo da imagem codificado em base64 (sem o prefixo data:).
    pub dados: Option<String>,
}

fn de_linha(r: &Row) -> rusqlite::Result<Foto> {
    Ok(Foto { id: r.get(0)?, veiculo_id: r.get(1)?, nome: r.get(2)?, mime: r.get(3)?, criado_em: r.get(4)? })
}

pub fn listar(conn: &Connection, veiculo_id: i64) -> AppResult<Vec<Foto>> {
    let mut stmt = conn.prepare(
        "SELECT id, veiculo_id, nome, mime, criado_em FROM fotos_veiculos WHERE veiculo_id = ?1 ORDER BY id",
    )?;
    let v = stmt.query_map([veiculo_id], de_linha)?.collect::<Result<Vec<_>, _>>()?;
    Ok(v)
}

pub fn adicionar(conn: &Connection, input: &FotoInput) -> AppResult<Foto> {
    let mut erros = Vec::new();
    let veiculo_id = input.veiculo_id.unwrap_or(0);
    if veiculo_id <= 0 {
        erros.push(FieldError { field: "veiculo_id".into(), message: "campo obrigatório".into() });
    }
    let mime = input.mime.clone().unwrap_or_default();
    if !TIPOS.contains(&mime.as_str()) {
        erros.push(FieldError { field: "foto".into(), message: "use uma imagem JPG, PNG ou WebP".into() });
    }
    let bytes = match input.dados.as_deref().map(|d| B64.decode(d)) {
        Some(Ok(b)) if !b.is_empty() => Some(b),
        _ => {
            erros.push(FieldError { field: "foto".into(), message: "arquivo vazio ou inválido".into() });
            None
        }
    };
    if let Some(b) = &bytes {
        if b.len() > TAMANHO_MAXIMO {
            erros.push(FieldError { field: "foto".into(), message: "a foto passa de 5 MB".into() });
        }
    }
    if !erros.is_empty() {
        return Err(AppError::Validation(erros));
    }
    crate::veiculos::buscar(conn, veiculo_id)?;
    let nome = input.nome.clone().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| "foto.jpg".into());
    conn.execute(
        "INSERT INTO fotos_veiculos (veiculo_id, nome, mime, dados, criado_em) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![veiculo_id, nome, mime, bytes.unwrap(), Utc::now().to_rfc3339()],
    )?;
    let id = conn.last_insert_rowid();
    Ok(conn.query_row(
        "SELECT id, veiculo_id, nome, mime, criado_em FROM fotos_veiculos WHERE id = ?1",
        [id],
        de_linha,
    )?)
}

/// Devolve a imagem em base64 para exibição.
pub fn dados(conn: &Connection, id: i64) -> AppResult<serde_json::Value> {
    let (mime, bytes): (String, Vec<u8>) = conn
        .query_row("SELECT mime, dados FROM fotos_veiculos WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(|_| AppError::NotFound("Foto não encontrada.".into()))?;
    Ok(serde_json::json!({ "mime": mime, "dados": B64.encode(bytes) }))
}

pub fn remover(conn: &Connection, id: i64) -> AppResult<()> {
    let n = conn.execute("DELETE FROM fotos_veiculos WHERE id = ?1", [id])?;
    if n == 0 {
        return Err(AppError::NotFound("Foto não encontrada.".into()));
    }
    Ok(())
}
