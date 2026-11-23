use rusqlite::{params, Connection, Row};
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::validation::Validator;

#[derive(Debug, Clone, Serialize)]
pub struct Cliente {
    pub id: i64,
    pub nome: String,
    pub telefone: Option<String>,
    pub email: Option<String>,
    pub cpf: Option<String>,
    pub endereco: Option<String>,
    pub observacoes: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ClienteInput {
    pub nome: Option<String>,
    pub telefone: Option<String>,
    pub email: Option<String>,
    pub cpf: Option<String>,
    pub endereco: Option<String>,
    pub observacoes: Option<String>,
}

const COLUNAS: &str = "id, nome, telefone, email, cpf, endereco, observacoes";

fn de_linha(r: &Row) -> rusqlite::Result<Cliente> {
    Ok(Cliente {
        id: r.get(0)?,
        nome: r.get(1)?,
        telefone: r.get(2)?,
        email: r.get(3)?,
        cpf: r.get(4)?,
        endereco: r.get(5)?,
        observacoes: r.get(6)?,
    })
}

fn validar(input: &ClienteInput) -> AppResult<ClienteInput> {
    let mut v = Validator::new();
    let limpo = ClienteInput {
        nome: v.text("nome", input.nome.as_deref(), true, 120),
        telefone: v.text("telefone", input.telefone.as_deref(), false, 30),
        email: v.text("email", input.email.as_deref(), false, 120),
        cpf: v.text("cpf", input.cpf.as_deref(), false, 14),
        endereco: v.text("endereco", input.endereco.as_deref(), false, 200),
        observacoes: v.text("observacoes", input.observacoes.as_deref(), false, 1000),
    };
    v.finish()?;
    Ok(limpo)
}

pub fn listar(conn: &Connection) -> AppResult<Vec<Cliente>> {
    let mut stmt = conn.prepare(&format!("SELECT {COLUNAS} FROM clientes ORDER BY nome"))?;
    let rows = stmt.query_map([], de_linha)?.collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn buscar(conn: &Connection, id: i64) -> AppResult<Cliente> {
    conn.query_row(&format!("SELECT {COLUNAS} FROM clientes WHERE id = ?1"), [id], de_linha)
        .map_err(|_| AppError::NotFound("Cliente não encontrado(a).".into()))
}

pub fn criar(conn: &Connection, input: &ClienteInput) -> AppResult<Cliente> {
    let d = validar(input)?;
    conn.execute(
        "INSERT INTO clientes (nome, telefone, email, cpf, endereco, observacoes) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![d.nome, d.telefone, d.email, d.cpf, d.endereco, d.observacoes],
    )?;
    buscar(conn, conn.last_insert_rowid())
}

pub fn atualizar(conn: &Connection, id: i64, input: &ClienteInput) -> AppResult<Cliente> {
    buscar(conn, id)?;
    let d = validar(input)?;
    conn.execute(
        "UPDATE clientes SET nome=?1, telefone=?2, email=?3, cpf=?4, endereco=?5, observacoes=?6 WHERE id=?7",
        params![d.nome, d.telefone, d.email, d.cpf, d.endereco, d.observacoes, id],
    )?;
    buscar(conn, id)
}

pub fn remover(conn: &Connection, id: i64) -> AppResult<()> {
    buscar(conn, id)?;
    conn.execute("DELETE FROM clientes WHERE id = ?1", [id])?;
    Ok(())
}
