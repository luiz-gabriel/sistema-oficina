use rusqlite::{params, Connection, Row};
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::seguranca::{hash_senha, validar_senha, verificar_senha};
use crate::validation::Validator;

#[derive(Debug, Clone, Serialize)]
pub struct Funcionario {
    pub id: i64,
    pub nome: String,
    pub cargo: Option<String>,
    pub telefone: Option<String>,
    /// Login de acesso ao sistema (None = funcionário cadastrado sem acesso).
    pub login: Option<String>,
    pub perfil: String,
    pub tem_acesso: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FuncionarioInput {
    pub nome: Option<String>,
    pub cargo: Option<String>,
    pub telefone: Option<String>,
}

/// Concessão de acesso: login, senha e perfil. Funcionário concede só perfil comum; administrador é regra de `comandos::pode_executar`.
/// `senha_hash` (argon2, já gerado) é aceito no lugar de `senha`: a fila offline e o envio ao servidor usam só o hash.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AcessoInput {
    pub login: Option<String>,
    pub senha: Option<String>,
    pub senha_hash: Option<String>,
    pub perfil: Option<String>,
}

fn de_linha(r: &Row) -> rusqlite::Result<Funcionario> {
    let login: Option<String> = r.get(4)?;
    let tem_acesso = r.get::<_, Option<String>>(5)?.is_some();
    Ok(Funcionario {
        id: r.get(0)?,
        nome: r.get(1)?,
        cargo: r.get(2)?,
        telefone: r.get(3)?,
        login,
        perfil: r.get(6)?,
        tem_acesso,
    })
}

const COLUNAS: &str = "id, nome, cargo, telefone, login, senha_hash, perfil";

fn validar(input: &FuncionarioInput) -> AppResult<FuncionarioInput> {
    let mut v = Validator::new();
    let limpo = FuncionarioInput {
        nome: v.text("nome", input.nome.as_deref(), true, 120),
        cargo: v.text("cargo", input.cargo.as_deref(), false, 60),
        telefone: v.text("telefone", input.telefone.as_deref(), false, 30),
    };
    v.finish()?;
    Ok(limpo)
}

pub fn listar(conn: &Connection) -> AppResult<Vec<Funcionario>> {
    let mut stmt = conn.prepare(&format!("SELECT {COLUNAS} FROM funcionarios ORDER BY nome"))?;
    let rows = stmt.query_map([], de_linha)?.collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn buscar(conn: &Connection, id: i64) -> AppResult<Funcionario> {
    conn.query_row(&format!("SELECT {COLUNAS} FROM funcionarios WHERE id = ?1"), [id], de_linha)
        .map_err(|_| AppError::NotFound("Funcionário não encontrado(a).".into()))
}

/// Cadastro não pede senha: o acesso é concedido depois, por `dar_acesso`.
pub fn criar(conn: &Connection, input: &FuncionarioInput) -> AppResult<Funcionario> {
    let d = validar(input)?;
    conn.execute(
        "INSERT INTO funcionarios (nome, cargo, telefone) VALUES (?1, ?2, ?3)",
        params![d.nome, d.cargo, d.telefone],
    )?;
    buscar(conn, conn.last_insert_rowid())
}

pub fn atualizar(conn: &Connection, id: i64, input: &FuncionarioInput) -> AppResult<Funcionario> {
    buscar(conn, id)?;
    let d = validar(input)?;
    conn.execute(
        "UPDATE funcionarios SET nome=?1, cargo=?2, telefone=?3 WHERE id=?4",
        params![d.nome, d.cargo, d.telefone, id],
    )?;
    buscar(conn, id)
}

pub fn remover(conn: &Connection, id: i64) -> AppResult<()> {
    buscar(conn, id)?;
    conn.execute("DELETE FROM funcionarios WHERE id = ?1", [id])?;
    Ok(())
}

pub fn dar_acesso(conn: &Connection, id: i64, input: &AcessoInput) -> AppResult<Funcionario> {
    buscar(conn, id)?;
    let mut v = Validator::new();
    let login = v.text("login", input.login.as_deref(), true, 30).map(|s| s.to_lowercase());
    let perfil = input.perfil.clone().unwrap_or_else(|| "funcionario".into());
    if perfil != "administrador" && perfil != "funcionario" {
        v.fail("perfil", "perfil desconhecido");
    }
    let senha_ja_hash = input.senha_hash.as_deref().filter(|h| !h.is_empty());
    let senha = input.senha.clone().unwrap_or_default();
    if let Some(l) = &login {
        if !l.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_') {
            v.fail("login", "use só letras, números, ponto ou underline");
        }
        // Com hash já pronto, a regra de senha foi aplicada antes, onde a senha estava em texto puro.
        if senha_ja_hash.is_none() {
            if let Err(AppError::Validation(e)) = validar_senha(&senha, l) {
                for err in e {
                    v.fail("senha", err.message);
                }
            }
        }
    }
    if let Some(h) = senha_ja_hash {
        if !h.starts_with("$argon2") {
            v.fail("senha", "hash de senha inválido");
        }
    }
    v.finish()?;
    let hash = match senha_ja_hash {
        Some(h) => h.to_string(),
        None => hash_senha(&senha)?,
    };
    conn.execute(
        "UPDATE funcionarios SET login=?1, senha_hash=?2, perfil=?3 WHERE id=?4",
        params![login, hash, perfil, id],
    )
    .map_err(|e| match AppError::from(e) {
        AppError::Conflict(_) => AppError::Conflict("Já existe um funcionário com esse login.".into()),
        outro => outro,
    })?;
    buscar(conn, id)
}

pub fn revogar_acesso(conn: &Connection, id: i64) -> AppResult<Funcionario> {
    buscar(conn, id)?;
    conn.execute("UPDATE funcionarios SET login=NULL, senha_hash=NULL, perfil='funcionario' WHERE id=?1", [id])?;
    buscar(conn, id)
}

/// Confere login e senha. Retorna o funcionário quando a senha confere.
pub fn autenticar(conn: &Connection, login: &str, senha: &str) -> AppResult<Option<Funcionario>> {
    let login = login.trim().to_lowercase();
    let hash: Option<String> = conn
        .query_row("SELECT senha_hash FROM funcionarios WHERE login = ?1", [&login], |r| r.get(0))
        .ok()
        .flatten();
    match hash {
        Some(h) if verificar_senha(senha, &h) => {
            let id: i64 = conn.query_row("SELECT id FROM funcionarios WHERE login = ?1", [&login], |r| r.get(0))?;
            Ok(Some(buscar(conn, id)?))
        }
        _ => Ok(None),
    }
}

/// Usuário administrador já existe?
pub fn existe_administrador(conn: &Connection) -> AppResult<bool> {
    let n: i64 = conn.query_row("SELECT COUNT(*) FROM funcionarios WHERE perfil = 'administrador' AND login IS NOT NULL", [], |r| r.get(0))?;
    Ok(n > 0)
}

/// Erros de senha seguidos até bloquear o acesso.
pub const MAX_TENTATIVAS: i64 = 3;
/// Tempo de bloqueio depois das tentativas erradas.
pub const MINUTOS_BLOQUEIO: i64 = 5;

/// Resultado de uma tentativa de entrada.
pub enum Entrada {
    Ok(Funcionario),
    /// Login ou senha errados. Não diz se o login existe nem quantas tentativas faltam.
    Falhou,
    /// Acesso bloqueado até o horário indicado (RFC 3339).
    Bloqueado { ate: String },
}

/// Entrada com limite de tentativas. As tentativas ficam numa tabela local (não sincroniza).
pub fn entrar(conn: &Connection, login: &str, senha: &str, agora: chrono::DateTime<chrono::Utc>) -> AppResult<Entrada> {
    let login = login.trim().to_lowercase();
    let hash: Option<String> = conn
        .query_row("SELECT senha_hash FROM funcionarios WHERE login = ?1", [&login], |r| r.get(0))
        .ok()
        .flatten();
    // Login que não existe: resposta igual à de senha errada, sem contar nem bloquear.
    let Some(hash) = hash else { return Ok(Entrada::Falhou) };

    let registro: Option<(i64, Option<String>)> = conn
        .query_row("SELECT falhas, bloqueado_ate FROM tentativas_login WHERE login = ?1", [&login], |r| Ok((r.get(0)?, r.get(1)?)))
        .ok();
    let mut falhas_antes = registro.as_ref().map(|(f, _)| *f).unwrap_or(0);
    if let Some((_, Some(ate))) = &registro {
        let ate_dt = chrono::DateTime::parse_from_rfc3339(ate)
            .map(|d| d.with_timezone(&chrono::Utc))
            .unwrap_or(agora);
        if ate_dt > agora {
            return Ok(Entrada::Bloqueado { ate: ate.clone() });
        }
        // Bloqueio já passou: começa a contar de novo.
        falhas_antes = 0;
        conn.execute("DELETE FROM tentativas_login WHERE login = ?1", [&login])?;
    }

    if verificar_senha(senha, &hash) {
        conn.execute("DELETE FROM tentativas_login WHERE login = ?1", [&login])?;
        let id: i64 = conn.query_row("SELECT id FROM funcionarios WHERE login = ?1", [&login], |r| r.get(0))?;
        return Ok(Entrada::Ok(buscar(conn, id)?));
    }

    let falhas = falhas_antes + 1;
    let bloqueado_ate = if falhas >= MAX_TENTATIVAS {
        Some((agora + chrono::Duration::minutes(MINUTOS_BLOQUEIO)).to_rfc3339())
    } else {
        None
    };
    conn.execute(
        "INSERT INTO tentativas_login (login, falhas, bloqueado_ate) VALUES (?1, ?2, ?3)
         ON CONFLICT(login) DO UPDATE SET falhas = excluded.falhas, bloqueado_ate = excluded.bloqueado_ate",
        params![login, falhas, bloqueado_ate],
    )?;
    match bloqueado_ate {
        Some(ate) => Ok(Entrada::Bloqueado { ate }),
        None => Ok(Entrada::Falhou),
    }
}
