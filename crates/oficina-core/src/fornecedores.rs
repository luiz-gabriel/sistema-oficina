//! Fornecedores: cadastro com CPF ou CNPJ validados pelos dígitos verificadores e documento único.
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::validation::Validator;

#[derive(Debug, Clone, Serialize)]
pub struct Fornecedor {
    pub id: i64,
    pub nome: String,
    /// CPF (11 dígitos) ou CNPJ (14 dígitos), só números.
    pub documento: String,
    pub telefone: Option<String>,
    pub email: Option<String>,
    pub endereco: Option<String>,
    pub observacoes: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct FornecedorInput {
    pub nome: Option<String>,
    pub documento: Option<String>,
    pub telefone: Option<String>,
    pub email: Option<String>,
    pub endereco: Option<String>,
    pub observacoes: Option<String>,
}

const COLUNAS: &str = "id, nome, documento, telefone, email, endereco, observacoes";

fn de_linha(r: &Row) -> rusqlite::Result<Fornecedor> {
    Ok(Fornecedor {
        id: r.get(0)?,
        nome: r.get(1)?,
        documento: r.get(2)?,
        telefone: r.get(3)?,
        email: r.get(4)?,
        endereco: r.get(5)?,
        observacoes: r.get(6)?,
    })
}

fn digitos(s: &str) -> Vec<u32> {
    s.bytes().filter(u8::is_ascii_digit).map(|b| u32::from(b - b'0')).collect()
}

/// CPF com 11 dígitos e dígitos verificadores corretos. Rejeita sequências iguais (111.111.111-11).
pub fn cpf_valido(entrada: &str) -> bool {
    let n = digitos(entrada);
    if n.len() != 11 || n.iter().all(|&x| x == n[0]) {
        return false;
    }
    let dv = |tam: usize| {
        let soma: u32 = (0..tam).map(|i| n[i] * (tam as u32 + 1 - i as u32)).sum();
        let r = soma * 10 % 11;
        if r == 10 { 0 } else { r }
    };
    dv(9) == n[9] && dv(10) == n[10]
}

const PESOS_CNPJ_1: [u32; 12] = [5, 4, 3, 2, 9, 8, 7, 6, 5, 4, 3, 2];
const PESOS_CNPJ_2: [u32; 13] = [6, 5, 4, 3, 2, 9, 8, 7, 6, 5, 4, 3, 2];

/// CNPJ com 14 dígitos e dígitos verificadores corretos. Rejeita sequências iguais.
pub fn cnpj_valido(entrada: &str) -> bool {
    let n = digitos(entrada);
    if n.len() != 14 || n.iter().all(|&x| x == n[0]) {
        return false;
    }
    let dv = |pesos: &[u32]| {
        let soma: u32 = pesos.iter().enumerate().map(|(i, p)| n[i] * p).sum();
        let r = soma % 11;
        if r < 2 { 0 } else { 11 - r }
    };
    dv(&PESOS_CNPJ_1) == n[12] && dv(&PESOS_CNPJ_2) == n[13]
}

/// Aceita CPF ou CNPJ com ou sem pontuação (123.456.789-09 ou 12.345.678/0001-95).
/// Devolve só os dígitos, que é a forma guardada no banco.
pub fn normalizar_documento(raw: &str) -> Result<String, String> {
    if !raw.chars().all(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '/' | ' ')) {
        return Err("use apenas números, pontos, traço e barra.".into());
    }
    let d: String = raw.chars().filter(|c| c.is_ascii_digit()).collect();
    match d.len() {
        11 if cpf_valido(&d) => Ok(d),
        14 if cnpj_valido(&d) => Ok(d),
        11 => Err("CPF inválido (os dígitos verificadores não conferem).".into()),
        14 => Err("CNPJ inválido (os dígitos verificadores não conferem).".into()),
        _ => Err("informe um CPF (11 números) ou um CNPJ (14 números).".into()),
    }
}

fn email_valido(e: &str) -> bool {
    match e.split_once('@') {
        Some((antes, depois)) => {
            !antes.is_empty() && !e.contains(' ') && depois.contains('.') && !depois.starts_with('.') && !depois.ends_with('.')
        }
        None => false,
    }
}

/// Campos já validados e limpos (documento só com números).
struct Limpo {
    nome: String,
    documento: String,
    telefone: Option<String>,
    email: Option<String>,
    endereco: Option<String>,
    observacoes: Option<String>,
}

fn validar(input: &FornecedorInput) -> AppResult<Limpo> {
    let mut v = Validator::new();
    let nome = v.text("nome", input.nome.as_deref(), true, 120);
    let documento = match input.documento.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        None => {
            v.fail("documento", "campo obrigatório");
            None
        }
        Some(raw) => match normalizar_documento(raw) {
            Ok(d) => Some(d),
            Err(msg) => {
                v.fail("documento", msg);
                None
            }
        },
    };
    let telefone = v.text("telefone", input.telefone.as_deref(), false, 30);
    let email = v.text("email", input.email.as_deref(), false, 120);
    if email.as_deref().is_some_and(|e| !email_valido(e)) {
        v.fail("email", "e-mail inválido");
    }
    let endereco = v.text("endereco", input.endereco.as_deref(), false, 200);
    let observacoes = v.text("observacoes", input.observacoes.as_deref(), false, 1000);
    v.finish()?;
    Ok(Limpo {
        nome: nome.expect("validado"),
        documento: documento.expect("validado"),
        telefone,
        email,
        endereco,
        observacoes,
    })
}

/// Documento único: dois fornecedores não podem ter o mesmo CPF/CNPJ (com ou sem pontuação).
fn garantir_documento_livre(conn: &Connection, documento: &str, ignorar_id: i64) -> AppResult<()> {
    let existe: Option<i64> = conn
        .query_row(
            "SELECT id FROM fornecedores WHERE documento = ?1 AND id <> ?2",
            params![documento, ignorar_id],
            |r| r.get(0),
        )
        .optional()?;
    if existe.is_some() {
        return Err(AppError::Validation(vec![crate::error::FieldError {
            field: "documento".into(),
            message: "já existe um fornecedor com este CPF/CNPJ.".into(),
        }]));
    }
    Ok(())
}

pub fn listar(conn: &Connection) -> AppResult<Vec<Fornecedor>> {
    let mut stmt = conn.prepare(&format!("SELECT {COLUNAS} FROM fornecedores ORDER BY nome"))?;
    let rows = stmt.query_map([], de_linha)?.collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn buscar(conn: &Connection, id: i64) -> AppResult<Fornecedor> {
    conn.query_row(&format!("SELECT {COLUNAS} FROM fornecedores WHERE id = ?1"), [id], de_linha)
        .optional()?
        .ok_or_else(|| AppError::NotFound("Fornecedor não encontrado.".into()))
}

/// Busca por nome (parte do texto) ou por documento (CPF/CNPJ, com ou sem pontuação).
/// Termo vazio devolve a lista completa.
pub fn buscar_por_termo(conn: &Connection, termo: &str) -> AppResult<Vec<Fornecedor>> {
    let t = termo.trim();
    if t.is_empty() {
        return listar(conn);
    }
    let escapado = t.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
    let padrao_nome = format!("%{escapado}%");
    let so_digitos: String = t.chars().filter(|c| c.is_ascii_digit()).collect();
    let padrao_doc = format!("%{so_digitos}%");
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUNAS} FROM fornecedores
         WHERE nome LIKE ?1 ESCAPE '\\' OR (?2 <> '' AND documento LIKE ?3)
         ORDER BY nome"
    ))?;
    let rows = stmt
        .query_map(params![padrao_nome, so_digitos, padrao_doc], de_linha)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn criar(conn: &Connection, input: &FornecedorInput) -> AppResult<Fornecedor> {
    let l = validar(input)?;
    garantir_documento_livre(conn, &l.documento, -1)?;
    conn.execute(
        "INSERT INTO fornecedores (nome, documento, telefone, email, endereco, observacoes)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![l.nome, l.documento, l.telefone, l.email, l.endereco, l.observacoes],
    )?;
    buscar(conn, conn.last_insert_rowid())
}

pub fn atualizar(conn: &Connection, id: i64, input: &FornecedorInput) -> AppResult<Fornecedor> {
    buscar(conn, id)?;
    let l = validar(input)?;
    garantir_documento_livre(conn, &l.documento, id)?;
    conn.execute(
        "UPDATE fornecedores SET nome=?1, documento=?2, telefone=?3, email=?4, endereco=?5, observacoes=?6
         WHERE id=?7",
        params![l.nome, l.documento, l.telefone, l.email, l.endereco, l.observacoes, id],
    )?;
    buscar(conn, id)
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn cpf_com_digitos_verificadores() {
        assert!(cpf_valido("52998224725"));
        assert!(!cpf_valido("52998224726"));
        assert!(!cpf_valido("11111111111"));
    }

    #[test]
    fn cnpj_com_digitos_verificadores() {
        assert!(cnpj_valido("11222333000181"));
        assert!(!cnpj_valido("11222333000182"));
        assert!(!cnpj_valido("00000000000000"));
    }
}
