//! Dispositivos autorizados, código de pareamento e pedidos de aprovação.
//!
//! Limite: 10 dispositivos ATIVOS, contando o computador principal. A vaga é ocupada só ao aprovar,
//! dentro de `BEGIN IMMEDIATE`, então pedidos simultâneos não ultrapassam o limite.
//!
//! Regra de administração (vale para toda revogação e para revogação de administrador):
//! deve restar pelo menos um administrador com acesso (login e senha) e pelo menos um computador ATIVO
//! (o principal ou outro aparelho). Assim o principal só pode ser revogado se já houver outro aparelho
//! autorizado; sem isso a revogação é recusada.
//!
//! Confirmação do pareamento: o código de 6 dígitos fica isolado nas funções `gerar_codigo` e
//! `conferir_codigo`. A troca por SPAKE2 (próxima etapa) deve mexer só nelas e em `enviar_pedido`.
//! Nenhum código, segredo ou token é gravado em claro nem registrado em log.
use chrono::{DateTime, Utc};
use rand::{Rng, RngCore};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::{json, Value};

use crate::comandos::Contexto;
use crate::error::{AppError, AppResult, FieldError};
use crate::seguranca::sha256_hex;
use crate::validation::Validator;

/// Máximo de dispositivos ATIVOS, contando o computador principal.
pub const LIMITE_DISPOSITIVOS: i64 = 10;
/// Validade do código de confirmação, em segundos.
pub const VALIDADE_CODIGO_SEGUNDOS: i64 = 5 * 60;
/// Erros de código até bloquear o código (mesmo o correto passa a ser recusado).
pub const MAX_TENTATIVAS_CODIGO: i64 = 3;

const MSG_LIMITE: &str = "Limite de 10 dispositivos ativos atingido. Revogue um dispositivo antes de autorizar outro.";
const MSG_ADMINISTRACAO: &str = "Esta alteração deixaria o sistema sem administração. Mantenha pelo menos um administrador com acesso e pelo menos um computador autorizado.";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Dispositivo {
    pub id: i64,
    pub nome: String,
    pub principal: bool,
    pub autorizado_em: Option<String>,
    pub ultimo_acesso: Option<String>,
    pub situacao: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PedidoPendente {
    pub id: i64,
    pub nome_aparelho: String,
    pub criado_em: String,
}

/// Código exibido ao administrador no computador principal.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CodigoPareamento {
    pub codigo: String,
    pub expira_em: i64,
}

/// Resposta ao aparelho que enviou o código: o segredo é usado para consultar o pedido depois.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PedidoEnviado {
    pub pedido_id: i64,
    pub segredo: String,
}

/// Situação de um pedido vista pelo aparelho. O token só aparece uma vez (na entrega).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Consulta {
    Pendente,
    Recusado,
    Liberado { token: String },
    JaEntregue,
}

fn agora_txt(agora: DateTime<Utc>) -> String {
    agora.to_rfc3339()
}

fn bytes_aleatorios<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    rand::thread_rng().fill_bytes(&mut b);
    b
}

/// Token de aparelho (32 bytes aleatórios em hexadecimal). Só o hash SHA-256 é guardado.
fn token_novo() -> String {
    hex::encode(bytes_aleatorios::<32>())
}

/// Executa `f` dentro de `BEGIN IMMEDIATE`: confirma se der certo, desfaz se der erro.
fn em_transacao<T>(conn: &Connection, f: impl FnOnce() -> AppResult<T>) -> AppResult<T> {
    conn.execute_batch("BEGIN IMMEDIATE")?;
    match f() {
        Ok(v) => match conn.execute_batch("COMMIT") {
            Ok(()) => Ok(v),
            Err(e) => {
                let _ = conn.execute_batch("ROLLBACK");
                Err(e.into())
            }
        },
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}

// ---------- Código de confirmação (trocável por SPAKE2 em etapa futura) ----------

fn gerar_codigo() -> String {
    format!("{:06}", rand::thread_rng().gen_range(0..1_000_000u32))
}

fn hash_codigo(sal: &str, codigo: &str) -> String {
    sha256_hex(&format!("{sal}:{codigo}"))
}

fn conferir_codigo(sal: &str, hash_guardado: &str, digitado: &str) -> bool {
    hash_codigo(sal, digitado) == hash_guardado
}

// ---------- Computador principal ----------

/// Garante a linha do computador principal (idempotente). Devolve o id, ATIVO ou REVOGADO.
pub fn garantir_principal(conn: &Connection, nome: &str, agora: DateTime<Utc>) -> AppResult<i64> {
    let existente: Option<i64> = conn
        .query_row("SELECT id FROM dispositivos WHERE principal = 1", [], |r| r.get(0))
        .optional()?;
    if let Some(id) = existente {
        return Ok(id);
    }
    conn.execute(
        "INSERT INTO dispositivos (nome, principal, criado_em, autorizado_em, situacao) VALUES (?1, 1, ?2, ?2, 'ATIVO')",
        params![nome, agora_txt(agora)],
    )?;
    Ok(conn.last_insert_rowid())
}

/// True se o dispositivo existe e está ATIVO.
pub fn esta_ativo(conn: &Connection, id: i64) -> AppResult<bool> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM dispositivos WHERE id = ?1 AND situacao = 'ATIVO'",
        [id],
        |r| r.get(0),
    )?;
    Ok(n > 0)
}

/// Identifica um aparelho pelo token guardado no pareamento. Devolve o id se o aparelho está ATIVO.
pub fn autenticar_dispositivo(conn: &Connection, token: &str, agora: DateTime<Utc>) -> AppResult<Option<i64>> {
    if token.is_empty() {
        return Ok(None);
    }
    let id: Option<i64> = conn
        .query_row(
            "SELECT id FROM dispositivos WHERE credencial_hash = ?1 AND situacao = 'ATIVO'",
            [sha256_hex(token)],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(id) = id {
        registrar_acesso(conn, id, agora)?;
    }
    Ok(id)
}

/// Marca o último acesso do dispositivo.
pub fn registrar_acesso(conn: &Connection, id: i64, agora: DateTime<Utc>) -> AppResult<()> {
    conn.execute("UPDATE dispositivos SET ultimo_acesso = ?1 WHERE id = ?2", params![agora_txt(agora), id])?;
    Ok(())
}

// ---------- Limite e administração ----------

fn contar_ativos(conn: &Connection) -> AppResult<i64> {
    Ok(conn.query_row("SELECT COUNT(*) FROM dispositivos WHERE situacao = 'ATIVO'", [], |r| r.get(0))?)
}

/// Regra de administração (ver cabeçalho do módulo). Chamada dentro da transação que faz a alteração.
fn garantir_administracao(conn: &Connection) -> AppResult<()> {
    let admins: i64 = conn.query_row(
        "SELECT COUNT(*) FROM funcionarios WHERE perfil = 'administrador' AND senha_hash IS NOT NULL",
        [],
        |r| r.get(0),
    )?;
    let computadores: i64 = conn.query_row(
        "SELECT COUNT(*) FROM dispositivos WHERE situacao = 'ATIVO'",
        [],
        |r| r.get(0),
    )?;
    if admins == 0 || computadores == 0 {
        return Err(AppError::Conflict(MSG_ADMINISTRACAO.into()));
    }
    Ok(())
}

// ---------- Listagens ----------

pub fn listar(conn: &Connection) -> AppResult<Vec<Dispositivo>> {
    let mut st = conn.prepare(
        "SELECT id, nome, principal, autorizado_em, ultimo_acesso, situacao FROM dispositivos ORDER BY principal DESC, id",
    )?;
    let linhas = st.query_map([], |r| {
        Ok(Dispositivo {
            id: r.get(0)?,
            nome: r.get(1)?,
            principal: r.get::<_, i64>(2)? == 1,
            autorizado_em: r.get(3)?,
            ultimo_acesso: r.get(4)?,
            situacao: r.get(5)?,
        })
    })?;
    Ok(linhas.collect::<Result<_, _>>()?)
}

pub fn listar_pedidos_pendentes(conn: &Connection) -> AppResult<Vec<PedidoPendente>> {
    let mut st = conn.prepare("SELECT id, nome_aparelho, criado_em FROM pedidos_pareamento WHERE situacao = 'PENDENTE' ORDER BY id")?;
    let linhas = st.query_map([], |r| Ok(PedidoPendente { id: r.get(0)?, nome_aparelho: r.get(1)?, criado_em: r.get(2)? }))?;
    Ok(linhas.collect::<Result<_, _>>()?)
}

// ---------- Pareamento ----------

/// Gera um código novo para exibir no principal. Códigos anteriores ainda não usados deixam de valer.
pub fn iniciar_pareamento(conn: &Connection, agora: DateTime<Utc>) -> AppResult<CodigoPareamento> {
    em_transacao(conn, || {
        conn.execute("UPDATE codigos_pareamento SET situacao = 'SUBSTITUIDO' WHERE situacao = 'ATIVO'", [])?;
        let codigo = gerar_codigo();
        let sal = hex::encode(bytes_aleatorios::<16>());
        let expira_em = agora.timestamp() + VALIDADE_CODIGO_SEGUNDOS;
        conn.execute(
            "INSERT INTO codigos_pareamento (sal, codigo_hash, criado_em, expira_em, tentativas_erradas, situacao)
             VALUES (?1, ?2, ?3, ?4, 0, 'ATIVO')",
            params![sal, hash_codigo(&sal, &codigo), agora_txt(agora), expira_em],
        )?;
        Ok(CodigoPareamento { codigo, expira_em })
    })
}

/// O aparelho digita o código e cria um pedido PENDENTE. O código vale uma vez.
/// Erros de código (errado, vencido, bloqueado) são gravados (contagem de tentativas) mesmo quando recusados.
pub fn enviar_pedido(conn: &Connection, codigo: &str, nome_aparelho: &str, agora: DateTime<Utc>) -> AppResult<PedidoEnviado> {
    let mut v = Validator::new();
    let nome = v.text("nome", Some(nome_aparelho), true, 60);
    v.finish()?;
    let nome = nome.unwrap_or_default();
    let digitado: String = codigo.chars().filter(|c| !c.is_whitespace()).collect();

    let resultado = em_transacao(conn, || {
        let ativo: Option<(i64, String, String, i64, i64)> = conn
            .query_row(
                "SELECT id, sal, codigo_hash, expira_em, tentativas_erradas FROM codigos_pareamento
                 WHERE situacao = 'ATIVO' ORDER BY id DESC LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()?;
        let Some((id, sal, hash, expira_em, erradas)) = ativo else {
            return Ok(Err(erro_codigo("Código inválido ou já usado. Gere um novo código no computador principal.")));
        };
        if agora.timestamp() >= expira_em {
            conn.execute("UPDATE codigos_pareamento SET situacao = 'EXPIRADO' WHERE id = ?1", [id])?;
            return Ok(Err(erro_codigo("Código vencido. Gere um novo código no computador principal.")));
        }
        if !conferir_codigo(&sal, &hash, &digitado) {
            let novas = erradas + 1;
            if novas >= MAX_TENTATIVAS_CODIGO {
                conn.execute(
                    "UPDATE codigos_pareamento SET tentativas_erradas = ?1, situacao = 'BLOQUEADO' WHERE id = ?2",
                    params![novas, id],
                )?;
                return Ok(Err(erro_codigo("Código bloqueado por excesso de tentativas. Gere um novo código no computador principal.")));
            }
            conn.execute("UPDATE codigos_pareamento SET tentativas_erradas = ?1 WHERE id = ?2", params![novas, id])?;
            let restam = MAX_TENTATIVAS_CODIGO - novas;
            return Ok(Err(erro_codigo(&format!("Código incorreto. Restam {restam} tentativa(s)."))));
        }
        conn.execute("UPDATE codigos_pareamento SET situacao = 'USADO' WHERE id = ?1", [id])?;
        let segredo = hex::encode(bytes_aleatorios::<32>());
        conn.execute(
            "INSERT INTO pedidos_pareamento (nome_aparelho, criado_em, situacao, segredo_hash)
             VALUES (?1, ?2, 'PENDENTE', ?3)",
            params![nome, agora_txt(agora), sha256_hex(&segredo)],
        )?;
        Ok(Ok(PedidoEnviado { pedido_id: conn.last_insert_rowid(), segredo }))
    })?;
    resultado
}

fn erro_codigo(mensagem: &str) -> AppError {
    AppError::Validation(vec![FieldError { field: "codigo".into(), message: mensagem.into() }])
}

/// Aprova o pedido e ocupa a vaga, tudo numa transação imediata. Recusa se o limite já foi atingido.
pub fn aprovar_pedido(conn: &Connection, pedido_id: i64, nome_principal: &str, agora: DateTime<Utc>) -> AppResult<Dispositivo> {
    em_transacao(conn, || {
        let situacao: Option<(String, String)> = conn
            .query_row(
                "SELECT situacao, nome_aparelho FROM pedidos_pareamento WHERE id = ?1",
                [pedido_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((situacao, nome)) = situacao else {
            return Err(AppError::NotFound("Pedido não encontrado.".into()));
        };
        if situacao != "PENDENTE" {
            return Err(AppError::Conflict("Este pedido já foi decidido.".into()));
        }
        garantir_principal(conn, nome_principal, agora)?;
        if contar_ativos(conn)? >= LIMITE_DISPOSITIVOS {
            return Err(AppError::Conflict(MSG_LIMITE.into()));
        }
        conn.execute(
            "INSERT INTO dispositivos (nome, principal, criado_em, autorizado_em, situacao)
             VALUES (?1, 0, ?2, ?2, 'ATIVO')",
            params![nome, agora_txt(agora)],
        )?;
        let id = conn.last_insert_rowid();
        conn.execute(
            "UPDATE pedidos_pareamento SET situacao = 'APROVADO', dispositivo_id = ?1, decidido_em = ?2 WHERE id = ?3",
            params![id, agora_txt(agora), pedido_id],
        )?;
        buscar(conn, id)
    })
}

pub fn recusar_pedido(conn: &Connection, pedido_id: i64, agora: DateTime<Utc>) -> AppResult<()> {
    em_transacao(conn, || {
        let situacao: Option<String> = conn
            .query_row("SELECT situacao FROM pedidos_pareamento WHERE id = ?1", [pedido_id], |r| r.get(0))
            .optional()?;
        match situacao.as_deref() {
            None => Err(AppError::NotFound("Pedido não encontrado.".into())),
            Some("PENDENTE") => {
                conn.execute(
                    "UPDATE pedidos_pareamento SET situacao = 'RECUSADO', decidido_em = ?1 WHERE id = ?2",
                    params![agora_txt(agora), pedido_id],
                )?;
                Ok(())
            }
            Some(_) => Err(AppError::Conflict("Este pedido já foi decidido.".into())),
        }
    })
}

/// O aparelho consulta o pedido com o segredo. Quando aprovado, recebe o token uma única vez.
pub fn consultar_pedido(conn: &Connection, pedido_id: i64, segredo: &str, agora: DateTime<Utc>) -> AppResult<Consulta> {
    let nao_achou = || AppError::NotFound("Pedido não encontrado.".into());
    em_transacao(conn, || {
        let linha: Option<(String, String, Option<i64>)> = conn
            .query_row(
                "SELECT situacao, segredo_hash, dispositivo_id FROM pedidos_pareamento WHERE id = ?1",
                [pedido_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let Some((situacao, segredo_hash, dispositivo_id)) = linha else { return Err(nao_achou()) };
        if sha256_hex(segredo) != segredo_hash {
            return Err(nao_achou());
        }
        match situacao.as_str() {
            "PENDENTE" => Ok(Consulta::Pendente),
            "RECUSADO" => Ok(Consulta::Recusado),
            "ENTREGUE" => Ok(Consulta::JaEntregue),
            "APROVADO" => {
                let id = dispositivo_id.ok_or_else(nao_achou)?;
                // Aparelho revogado depois da aprovação não recebe credencial.
                if !esta_ativo(conn, id)? {
                    return Err(nao_achou());
                }
                let token = token_novo();
                conn.execute(
                    "UPDATE dispositivos SET credencial_hash = ?1 WHERE id = ?2 AND situacao = 'ATIVO'",
                    params![sha256_hex(&token), id],
                )?;
                conn.execute("UPDATE pedidos_pareamento SET situacao = 'ENTREGUE' WHERE id = ?1", [pedido_id])?;
                registrar_acesso(conn, id, agora)?;
                Ok(Consulta::Liberado { token })
            }
            _ => Err(nao_achou()),
        }
    })
}

/// Revoga um dispositivo: sai de ATIVO (libera a vaga), perde a credencial e todas as sessões.
/// Recusa a revogação se deixar o sistema sem administração (ver `garantir_administracao`).
pub fn revogar(conn: &Connection, id: i64) -> AppResult<()> {
    em_transacao(conn, || {
        let situacao: Option<String> = conn
            .query_row("SELECT situacao FROM dispositivos WHERE id = ?1", [id], |r| r.get(0))
            .optional()?;
        match situacao.as_deref() {
            None => return Err(AppError::NotFound("Dispositivo não encontrado.".into())),
            Some("ATIVO") => {}
            Some(_) => return Err(AppError::Conflict("Este dispositivo já está revogado.".into())),
        }
        conn.execute("UPDATE dispositivos SET situacao = 'REVOGADO', credencial_hash = NULL WHERE id = ?1", [id])?;
        conn.execute("DELETE FROM sessoes WHERE dispositivo_id = ?1", [id])?;
        garantir_administracao(conn)
    })
}

fn buscar(conn: &Connection, id: i64) -> AppResult<Dispositivo> {
    listar(conn)?
        .into_iter()
        .find(|d| d.id == id)
        .ok_or_else(|| AppError::NotFound("Dispositivo não encontrado.".into()))
}

// ---------- Comandos (mesmo despacho de `comandos::executar`) ----------

fn id_do_args(args: &Value) -> AppResult<i64> {
    args.get("id").and_then(Value::as_i64).ok_or_else(|| AppError::Validation(vec![FieldError {
        field: "id".into(),
        message: "informe o identificador".into(),
    }]))
}

/// Comandos de dispositivos e pareamento. Só o administrador; a verificação também fica em `comandos::pode_executar`.
pub fn executar(conn: &Connection, cmd: &str, args: &Value, ctx: &Contexto) -> AppResult<Value> {
    if ctx.perfil != crate::comandos::ADMINISTRADOR {
        return Err(AppError::Conflict("Só o administrador pode gerenciar dispositivos.".into()));
    }
    let agora = Utc::now();
    // O computador principal aparece na lista desde o primeiro uso da tela, mesmo sem login web.
    garantir_principal(conn, &ctx.computador, agora)?;
    let valor = match cmd {
        "dispositivos_listar" => json!(listar(conn)?),
        "dispositivos_iniciar_pareamento" => json!(iniciar_pareamento(conn, agora)?),
        "pareamento_listar_pedidos" => json!(listar_pedidos_pendentes(conn)?),
        "pareamento_aprovar" => json!(aprovar_pedido(conn, id_do_args(args)?, &ctx.computador, agora)?),
        "pareamento_recusar" => {
            recusar_pedido(conn, id_do_args(args)?, agora)?;
            Value::Null
        }
        "dispositivos_revogar" => {
            revogar(conn, id_do_args(args)?)?;
            Value::Null
        }
        _ => return Err(AppError::NotFound(format!("comando desconhecido: {cmd}"))),
    };
    Ok(valor)
}
