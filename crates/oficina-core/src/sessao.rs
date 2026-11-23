//! Última sessão de login deste computador, para a tela "Continuar como <nome>".
//! Fica na tabela `config` (não sincroniza). Guarda só usuário, nome, perfil e horário do login:
//! nunca a senha, o hash nem o token de rede.
use chrono::{DateTime, Duration, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::config;
use crate::error::AppResult;
use crate::funcionarios::{self, Funcionario};

const CHAVE: &str = "ultima_sessao";

/// Até quantas horas depois do login a tela oferece "Continuar como".
pub const JANELA_HORAS: i64 = 2;

/// Dados guardados do último login. Os campos são só estes: não há lugar para senha ou token.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessaoGuardada {
    pub usuario: String,
    pub nome: String,
    pub perfil: String,
    /// Horário do login (RFC 3339, UTC).
    pub em: String,
}

/// Guarda o login que acabou de acontecer. Recebe o funcionário já autenticado, sem senha.
pub fn guardar(conn: &Connection, f: &Funcionario, agora: DateTime<Utc>) -> AppResult<()> {
    let dados = SessaoGuardada {
        usuario: f.login.clone().unwrap_or_default(),
        nome: f.nome.clone(),
        perfil: f.perfil.clone(),
        em: agora.to_rfc3339(),
    };
    let texto = serde_json::to_string(&dados).map_err(|e| crate::error::AppError::Database(e.to_string()))?;
    config::definir(conn, CHAVE, &texto)
}

/// Lê a sessão guardada. Valor ilegível é tratado como ausência.
pub fn ler(conn: &Connection) -> AppResult<Option<SessaoGuardada>> {
    Ok(config::obter(conn, CHAVE)?.and_then(|t| serde_json::from_str(&t).ok()))
}

/// Apaga a sessão guardada (feito ao sair).
pub fn apagar(conn: &Connection) -> AppResult<()> {
    conn.execute("DELETE FROM config WHERE chave = ?1", params![CHAVE])?;
    Ok(())
}

/// Minutos decorridos desde `em`, se estiver dentro da janela de até `JANELA_HORAS`.
/// None quando passou da janela, quando o horário é futuro (relógio mudou) ou quando é ilegível.
pub fn minutos_na_janela(em: &str, agora: DateTime<Utc>) -> Option<i64> {
    let em = DateTime::parse_from_rfc3339(em).ok()?.with_timezone(&Utc);
    let decorrido = agora - em;
    if decorrido < Duration::zero() || decorrido > Duration::hours(JANELA_HORAS) {
        return None;
    }
    Some(decorrido.num_minutes())
}

/// A sessão guardada que a tela de login pode oferecer agora, com os minutos desde o login.
/// Só vale se: a sessão está na janela, o usuário ainda tem acesso e o perfil vem do cadastro atual.
pub fn para_continuar(conn: &Connection, agora: DateTime<Utc>) -> AppResult<Option<(Funcionario, i64)>> {
    let Some(guardada) = ler(conn)? else { return Ok(None) };
    let Some(minutos) = minutos_na_janela(&guardada.em, agora) else { return Ok(None) };
    let id: Option<i64> = conn
        .query_row("SELECT id FROM funcionarios WHERE login = ?1 AND senha_hash IS NOT NULL", [&guardada.usuario], |r| r.get(0))
        .optional()?;
    match id {
        Some(id) => Ok(Some((funcionarios::buscar(conn, id)?, minutos))),
        None => Ok(None),
    }
}
