//! Registro de alterações: cada gravação recebe UUID, computador, usuário, data/hora e status de sincronização.
//! Também é a fila de envio do modo offline (status `pendente`) e guarda os conflitos a resolver.
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::Serialize;
use serde_json::{json, Value};

use crate::comandos::{self, Contexto};
use crate::error::{AppError, AppResult};
use crate::seguranca::{hash_senha, validar_senha};

#[derive(Debug, Clone, Serialize)]
pub struct Alteracao {
    pub uuid: String,
    pub criado_em: String,
    pub computador: String,
    pub usuario: String,
    pub comando: String,
    pub args: Value,
    pub tabela: Option<String>,
    pub id_local: Option<i64>,
    pub base_hash: Option<String>,
    pub forcar: bool,
    pub status: String,
    pub erro: Option<String>,
    pub conflito: Option<Value>,
}

fn de_linha(r: &Row) -> rusqlite::Result<Alteracao> {
    let args: String = r.get(5)?;
    let conflito: Option<String> = r.get(12)?;
    Ok(Alteracao {
        uuid: r.get(0)?,
        criado_em: r.get(1)?,
        computador: r.get(2)?,
        usuario: r.get(3)?,
        comando: r.get(4)?,
        args: serde_json::from_str(&args).unwrap_or(Value::Null),
        tabela: r.get(6)?,
        id_local: r.get(7)?,
        base_hash: r.get(8)?,
        forcar: r.get::<_, i64>(9)? != 0,
        status: r.get(10)?,
        erro: r.get(11)?,
        conflito: conflito.and_then(|c| serde_json::from_str(&c).ok()),
    })
}

const COLUNAS: &str = "uuid, criado_em, computador, usuario, comando, args, tabela, id_local, base_hash, forcar, status, erro, conflito";

pub struct Nova<'a> {
    pub computador: &'a str,
    pub usuario: &'a str,
    pub comando: &'a str,
    pub args: &'a Value,
    pub tabela: Option<&'a str>,
    pub id_local: Option<i64>,
    pub base_hash: Option<&'a str>,
    pub status: &'a str,
}

/// Identificador de uma operação: o mesmo é enviado ao servidor e guardado na fila, para não aplicar duas vezes.
pub fn novo_uuid() -> String {
    uuid::Uuid::new_v4().to_string()
}

pub fn registrar(conn: &Connection, n: &Nova) -> AppResult<String> {
    registrar_com_uuid(conn, n, &novo_uuid())
}

pub fn registrar_com_uuid(conn: &Connection, n: &Nova, uuid: &str) -> AppResult<String> {
    conn.execute(
        "INSERT INTO alteracoes (uuid, criado_em, computador, usuario, comando, args, tabela, id_local, base_hash, status)
         VALUES (?1, datetime('now'), ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![uuid, n.computador, n.usuario, n.comando, args_guardados(n.args, n.status).to_string(), n.tabela, n.id_local, n.base_hash, n.status],
    )?;
    Ok(uuid.to_string())
}

pub fn listar(conn: &Connection, status: &[&str]) -> AppResult<Vec<Alteracao>> {
    let marcas = status.iter().map(|s| format!("'{s}'")).collect::<Vec<_>>().join(",");
    let mut stmt = conn.prepare(&format!("SELECT {COLUNAS} FROM alteracoes WHERE status IN ({marcas}) ORDER BY seq"))?;
    let v = stmt.query_map([], de_linha)?.collect::<Result<Vec<_>, _>>()?;
    Ok(v)
}

pub fn historico(conn: &Connection, limite: i64) -> AppResult<Vec<Alteracao>> {
    let mut stmt = conn.prepare(&format!("SELECT {COLUNAS} FROM alteracoes ORDER BY seq DESC LIMIT ?1"))?;
    let v = stmt.query_map([limite], de_linha)?.collect::<Result<Vec<_>, _>>()?;
    Ok(v)
}

pub fn contar_pendentes(conn: &Connection) -> AppResult<i64> {
    Ok(conn.query_row("SELECT COUNT(*) FROM alteracoes WHERE status IN ('pendente', 'conflito', 'erro')", [], |r| r.get(0))?)
}

/// Senha só fica guardada enquanto a alteração ainda precisa ser enviada ao servidor, e nunca em texto puro:
/// quem chega aqui já trocou a senha pelo hash (ver `preparar_args`).
fn args_guardados(args: &Value, status: &str) -> Value {
    if status == "pendente" { args.clone() } else { crate::registro::sem_segredos(args) }
}

/// Prepara os argumentos de um comando antes de executar, enfileirar ou enviar.
/// Senha em texto puro é validada aqui e trocada pelo hash (`senha_hash`): a fila, o envio e o servidor só veem o hash.
pub fn preparar_args(comando: &str, args: &Value) -> AppResult<Value> {
    let mut a = args.clone();
    if comando == "funcionarios_dar_acesso" {
        tirar_senha_em_claro(&mut a, true)?;
    }
    Ok(a)
}

/// Se `args.dados.senha` vier em texto puro, grava `senha_hash` no lugar. Com `validar`, aplica a regra de senha antes.
/// Devolve se algo foi trocado.
fn tirar_senha_em_claro(args: &mut Value, validar: bool) -> AppResult<bool> {
    let Some(dados) = args.get_mut("dados").and_then(Value::as_object_mut) else { return Ok(false) };
    let Some(senha) = dados.get("senha").and_then(Value::as_str).map(str::to_string) else { return Ok(false) };
    if validar {
        let login = dados.get("login").and_then(Value::as_str).unwrap_or("").to_string();
        validar_senha(&senha, &login)?;
    }
    let hash = hash_senha(&senha)?;
    dados.remove("senha");
    dados.insert("senha_hash".into(), Value::String(hash));
    Ok(true)
}

/// Filas gravadas antes desta regra podem ter senha em texto puro. Troca por hash (a senha já passou pela
/// validação quando foi digitada). Devolve quantas linhas foram alteradas.
pub fn migrar_senhas_em_claro(conn: &Connection) -> AppResult<usize> {
    let linhas: Vec<(String, String, Option<String>)> = {
        let mut stmt = conn.prepare(
            "SELECT uuid, args, conflito FROM alteracoes
             WHERE comando = 'funcionarios_dar_acesso' AND status IN ('pendente', 'conflito', 'erro')",
        )?;
        let v = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<Result<Vec<_>, _>>()?;
        v
    };
    let mut trocadas = 0;
    for (uuid, args_txt, conflito_txt) in linhas {
        let mut args: Value = serde_json::from_str(&args_txt).unwrap_or(Value::Null);
        let mut mudou = tirar_senha_em_claro(&mut args, false)?;
        let mut conflito = conflito_txt.as_deref().and_then(|t| serde_json::from_str::<Value>(t).ok());
        if let Some(local) = conflito.as_mut().and_then(|c| c.get_mut("local")) {
            mudou |= tirar_senha_em_claro(local, false)?;
        }
        if mudou {
            let novo_conflito = match (&conflito, &conflito_txt) {
                (Some(c), _) => Some(c.to_string()),
                (None, Some(original)) => Some(original.clone()),
                (None, None) => None,
            };
            conn.execute(
                "UPDATE alteracoes SET args = ?1, conflito = ?2 WHERE uuid = ?3",
                params![args.to_string(), novo_conflito, uuid],
            )?;
            trocadas += 1;
        }
    }
    Ok(trocadas)
}

/// Refaz, sobre a cópia recém-recebida do servidor, as alterações que o servidor ainda não aceitou
/// (pendente, conflito, erro), na ordem em que aconteceram. Quem não couber mais (o registro sumiu, por exemplo)
/// é pulado e continua na fila com o status que já tinha. Devolve quantas foram refeitas.
pub fn reaplicar_sobre(conn: &Connection) -> AppResult<usize> {
    let mut refeitas = 0;
    for item in listar(conn, &["pendente", "conflito", "erro"])? {
        let args = match comandos::remapear(&item.comando, &item.args, &|t, id| match situacao_id(conn, t, id)? {
            None => Ok(None),
            Some(Some(s)) => Ok(Some(s)),
            Some(None) => Err(AppError::Conflict("depende de um registro que não está mais na cópia local".into())),
        }) {
            Ok(a) => a,
            Err(_) => continue,
        };
        let ctx = Contexto { usuario: item.usuario.clone(), perfil: "administrador".into(), computador: item.computador.clone() };
        // Savepoint: um comando que falha no meio não deixa escrita parcial na cópia.
        conn.execute_batch("SAVEPOINT reaplicacao")?;
        match comandos::executar(conn, &item.comando, &args, &ctx) {
            Ok(_) => {
                conn.execute_batch("RELEASE reaplicacao")?;
                refeitas += 1;
            }
            Err(_) => conn.execute_batch("ROLLBACK TO reaplicacao; RELEASE reaplicacao")?,
        }
    }
    Ok(refeitas)
}

pub fn atualizar(conn: &Connection, uuid: &str, status: &str, erro: Option<&str>, conflito: Option<&Value>) -> AppResult<()> {
    if status == "sincronizado" || status == "descartado" {
        // Já foi enviada (ou descartada): apaga a senha que estava na fila.
        let antigo: Option<String> = conn.query_row("SELECT args FROM alteracoes WHERE uuid = ?1", [uuid], |r| r.get(0)).ok();
        if let Some(v) = antigo.and_then(|t| serde_json::from_str::<Value>(&t).ok()) {
            conn.execute("UPDATE alteracoes SET args = ?1 WHERE uuid = ?2", params![crate::registro::sem_segredos(&v).to_string(), uuid])?;
        }
    }
    conn.execute(
        "UPDATE alteracoes SET status = ?1, erro = ?2, conflito = ?3 WHERE uuid = ?4",
        params![status, erro, conflito.map(|c| c.to_string()), uuid],
    )?;
    Ok(())
}

/// Resolve um conflito: "servidor" descarta a alteração local; "local" reenvia forçando a versão local.
pub fn resolver(conn: &Connection, uuid: &str, escolha: &str) -> AppResult<()> {
    match escolha {
        "servidor" => atualizar(conn, uuid, "descartado", None, None),
        _ => {
            conn.execute(
                "UPDATE alteracoes SET status = 'pendente', base_hash = NULL, forcar = 1, erro = NULL, conflito = NULL WHERE uuid = ?1",
                [uuid],
            )?;
            Ok(())
        }
    }
}

// ---- IDs de registros criados offline -------------------------------------------------

pub fn registrar_id_local(conn: &Connection, tabela: &str, id_local: i64) -> AppResult<()> {
    conn.execute(
        "INSERT OR IGNORE INTO ids_locais (tabela, id_local, id_servidor) VALUES (?1, ?2, NULL)",
        params![tabela, id_local],
    )?;
    Ok(())
}

pub fn definir_id_servidor(conn: &Connection, tabela: &str, id_local: i64, id_servidor: i64) -> AppResult<()> {
    conn.execute(
        "UPDATE ids_locais SET id_servidor = ?3 WHERE tabela = ?1 AND id_local = ?2",
        params![tabela, id_local, id_servidor],
    )?;
    Ok(())
}

/// Id do servidor para um id local: `Some(None)` = criado offline e ainda não enviado; `None` = id já é do servidor.
pub fn situacao_id(conn: &Connection, tabela: &str, id: i64) -> AppResult<Option<Option<i64>>> {
    let r: Option<Option<i64>> = {
        use rusqlite::OptionalExtension;
        conn.query_row(
            "SELECT id_servidor FROM ids_locais WHERE tabela = ?1 AND id_local = ?2",
            params![tabela, id],
            |r| r.get(0),
        )
        .optional()?
    };
    Ok(r)
}

pub fn limpar_ids_locais(conn: &Connection) -> AppResult<()> {
    conn.execute("DELETE FROM ids_locais", [])?;
    Ok(())
}

// ---- Envio com identificador único (idempotência) ----------------------------------------

/// Mensagem interna de `preparar_envio` quando a alteração depende de um registro criado offline ainda não enviado.
const DEPENDENCIA_PENDENTE: &str = "Depende de um registro que ainda não foi enviado ao servidor.";

/// Registra no servidor uma alteração já aplicada, com o identificador que o cliente enviou.
/// O resultado do comando fica guardado (na coluna `conflito`, sob a chave `resultado`) para responder
/// a um reenvio da mesma alteração sem aplicá-la de novo.
pub fn registrar_aplicada(conn: &Connection, uuid: &str, n: &Nova, resultado: &Value) -> AppResult<()> {
    conn.execute(
        "INSERT INTO alteracoes (uuid, criado_em, computador, usuario, comando, args, tabela, id_local, base_hash, status, conflito)
         VALUES (?1, datetime('now'), ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'sincronizado', ?9)",
        params![
            uuid, n.computador, n.usuario, n.comando, tirar_segredos(n.args).to_string(), n.tabela, n.id_local, n.base_hash,
            json!({"resultado": resultado}).to_string()
        ],
    )?;
    Ok(())
}

/// Se o servidor já aplicou a alteração com este identificador, devolve o resultado que ela teve.
pub fn resultado_aplicado(conn: &Connection, uuid: &str) -> AppResult<Option<Value>> {
    let linha: Option<(String, Option<String>)> = conn
        .query_row("SELECT status, conflito FROM alteracoes WHERE uuid = ?1", [uuid], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?;
    Ok(match linha {
        Some((status, Some(c))) if status == "sincronizado" => {
            serde_json::from_str::<Value>(&c).ok().and_then(|v| v.get("resultado").cloned())
        }
        _ => None,
    })
}

/// Prepara uma alteração pendente para envio: troca os ids locais pelos ids do servidor.
/// `Ok(None)` = ela depende de um registro criado offline que ainda não foi aceito pelo servidor.
/// Nesse caso a alteração continua pendente, sem mudar de estado, e é enviada depois da dependência.
pub fn preparar_envio(conn: &Connection, item: &Alteracao) -> AppResult<Option<Value>> {
    let resultado = comandos::remapear(&item.comando, &item.args, &|t, id| match situacao_id(conn, t, id)? {
        None => Ok(None),
        Some(Some(s)) => Ok(Some(s)),
        Some(None) => Err(AppError::Conflict(DEPENDENCIA_PENDENTE.into())),
    });
    match resultado {
        Ok(a) => Ok(Some(a)),
        Err(AppError::Conflict(m)) if m == DEPENDENCIA_PENDENTE => Ok(None),
        Err(e) => Err(e),
    }
}

/// Tira senhas e chaves de um valor, em todos os níveis (o campo some, não vira texto).
fn tirar_segredos(v: &Value) -> Value {
    match v {
        Value::Object(m) => Value::Object(
            m.iter()
                .filter(|(k, _)| !matches!(k.as_str(), "senha" | "senha_hash" | "chave"))
                .map(|(k, val)| (k.clone(), tirar_segredos(val)))
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.iter().map(tirar_segredos).collect()),
        outro => outro.clone(),
    }
}

/// Um campo que difere entre a versão local e a do servidor.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Diferenca {
    pub campo: String,
    pub local: Value,
    pub servidor: Value,
}

/// As duas versões de um conflito, lado a lado, sem senhas. `servidor` é Null se o registro foi removido.
#[derive(Debug, Clone, Serialize)]
pub struct VersoesConflito {
    pub uuid: String,
    pub comando: String,
    pub local: Value,
    pub servidor: Value,
    pub diferencas: Vec<Diferenca>,
}

/// Versão local e versão do servidor de um conflito, para o administrador escolher qual manter.
/// Só leitura: a escolha continua sendo feita por `resolver`.
pub fn versoes_do_conflito(conn: &Connection, uuid: &str) -> AppResult<VersoesConflito> {
    let linha: Option<(String, String, Option<String>)> = conn
        .query_row(
            "SELECT comando, args, conflito FROM alteracoes WHERE uuid = ?1 AND status = 'conflito'",
            [uuid],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let Some((comando, args_txt, conflito_txt)) = linha else {
        return Err(AppError::NotFound("Conflito não encontrado.".into()));
    };
    let args: Value = serde_json::from_str(&args_txt).unwrap_or(Value::Null);
    let conflito: Value = conflito_txt.and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
    // O que o usuário informou: os campos de `dados` (ou os próprios args, se não houver `dados`).
    let local = tirar_segredos(args.get("dados").unwrap_or(&args));
    let servidor = tirar_segredos(conflito.get("servidor").unwrap_or(&Value::Null));
    let mut diferencas = Vec::new();
    if let (Some(campos), Some(atual)) = (local.as_object(), servidor.as_object()) {
        for (campo, valor) in campos {
            if campo == "id" {
                continue;
            }
            let do_servidor = atual.get(campo).cloned().unwrap_or(Value::Null);
            if do_servidor != *valor {
                diferencas.push(Diferenca { campo: campo.clone(), local: valor.clone(), servidor: do_servidor });
            }
        }
    }
    Ok(VersoesConflito { uuid: uuid.to_string(), comando, local, servidor, diferencas })
}
