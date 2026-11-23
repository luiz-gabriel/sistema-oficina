use rusqlite::{params, Connection, Row};
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::validation::Validator;

/// Peças e serviços na mesma tabela; `tipo` diferencia. Só peças têm estoque relevante.
#[derive(Debug, Clone, Serialize)]
pub struct Item {
    pub id: i64,
    pub nome: String,
    pub tipo: String,
    pub codigo: Option<String>,
    pub preco_custo: f64,
    pub preco_venda: f64,
    pub quantidade: i64,
    pub estoque_minimo: i64,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ItemInput {
    pub nome: Option<String>,
    pub tipo: Option<String>,
    pub codigo: Option<String>,
    pub preco_custo: Option<f64>,
    pub preco_venda: Option<f64>,
    pub quantidade: Option<i64>,
    pub estoque_minimo: Option<i64>,
}

const COLUNAS: &str = "id, nome, tipo, codigo, preco_custo, preco_venda, quantidade, estoque_minimo";

fn de_linha(r: &Row) -> rusqlite::Result<Item> {
    Ok(Item {
        id: r.get(0)?,
        nome: r.get(1)?,
        tipo: r.get(2)?,
        codigo: r.get(3)?,
        preco_custo: r.get(4)?,
        preco_venda: r.get(5)?,
        quantidade: r.get(6)?,
        estoque_minimo: r.get(7)?,
    })
}

fn validar(input: &ItemInput) -> AppResult<(String, String, Option<String>, f64, f64, i64, i64)> {
    let mut v = Validator::new();
    let nome = v.text("nome", input.nome.as_deref(), true, 120);
    let tipo = match input.tipo.as_deref() {
        Some(t @ ("peca" | "servico")) => Some(t.to_string()),
        Some(_) => {
            v.fail("tipo", "deve ser 'peca' ou 'servico'");
            None
        }
        None => {
            v.fail("tipo", "campo obrigatório");
            None
        }
    };
    let codigo = v.text("codigo", input.codigo.as_deref(), false, 40);
    let custo = v.money("preco_custo", input.preco_custo, false).unwrap_or(0.0);
    let venda = v.money("preco_venda", input.preco_venda, true);
    let quantidade = v.int("quantidade", input.quantidade, false, 0, i64::MAX).unwrap_or(0);
    let minimo = v.int("estoque_minimo", input.estoque_minimo, false, 0, i64::MAX).unwrap_or(0);
    v.finish()?;
    Ok((nome.expect("validado"), tipo.expect("validado"), codigo, custo, venda.expect("validado"), quantidade, minimo))
}

pub fn listar(conn: &Connection) -> AppResult<Vec<Item>> {
    let mut stmt = conn.prepare(&format!("SELECT {COLUNAS} FROM itens ORDER BY nome"))?;
    let rows = stmt.query_map([], de_linha)?.collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn buscar(conn: &Connection, id: i64) -> AppResult<Item> {
    conn.query_row(&format!("SELECT {COLUNAS} FROM itens WHERE id = ?1"), [id], de_linha)
        .map_err(|_| AppError::NotFound("Item não encontrado.".into()))
}

/// Busca exata pelo código de barras (o leitor digita o código inteiro e manda Enter).
pub fn buscar_por_codigo(conn: &Connection, codigo: &str) -> AppResult<Item> {
    let codigo = codigo.trim();
    if codigo.is_empty() {
        return Err(AppError::NotFound("Leia ou digite o código.".into()));
    }
    conn.query_row(&format!("SELECT {COLUNAS} FROM itens WHERE codigo = ?1"), [codigo], de_linha)
        .map_err(|_| AppError::NotFound(format!("Nenhuma peça ou serviço com o código {codigo}.")))
}

/// Dois itens não podem ter o mesmo código: senão a leitura não saberia qual escolher.
fn garantir_codigo_livre(conn: &Connection, codigo: &Option<String>, ignorar_id: i64) -> AppResult<()> {
    let Some(codigo) = codigo else { return Ok(()) };
    let existe: Option<i64> = conn
        .query_row("SELECT id FROM itens WHERE codigo = ?1 AND id != ?2", params![codigo, ignorar_id], |r| r.get(0))
        .ok();
    if existe.is_some() {
        return Err(AppError::Validation(vec![crate::error::FieldError {
            field: "codigo".into(),
            message: "Já existe uma peça ou serviço com este código.".into(),
        }]));
    }
    Ok(())
}

pub fn criar(conn: &Connection, input: &ItemInput) -> AppResult<Item> {
    let (nome, tipo, codigo, custo, venda, qtd, minimo) = validar(input)?;
    garantir_codigo_livre(conn, &codigo, -1)?;
    conn.execute(
        "INSERT INTO itens (nome, tipo, codigo, preco_custo, preco_venda, quantidade, estoque_minimo)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![nome, tipo, codigo, custo, venda, qtd, minimo],
    )?;
    buscar(conn, conn.last_insert_rowid())
}

pub fn atualizar(conn: &Connection, id: i64, input: &ItemInput) -> AppResult<Item> {
    buscar(conn, id)?;
    let (nome, tipo, codigo, custo, venda, qtd, minimo) = validar(input)?;
    garantir_codigo_livre(conn, &codigo, id)?;
    conn.execute(
        "UPDATE itens SET nome=?1, tipo=?2, codigo=?3, preco_custo=?4, preco_venda=?5, quantidade=?6, estoque_minimo=?7
         WHERE id=?8",
        params![nome, tipo, codigo, custo, venda, qtd, minimo, id],
    )?;
    buscar(conn, id)
}

pub fn remover(conn: &Connection, id: i64) -> AppResult<()> {
    buscar(conn, id)?;
    conn.execute("DELETE FROM itens WHERE id = ?1", [id])?;
    Ok(())
}
