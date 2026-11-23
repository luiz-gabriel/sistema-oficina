//! Sincronização: snapshot do banco, identificação de linhas (hash), remapeamento de IDs locais.
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use rusqlite::types::Value as SqlValue;
use rusqlite::{params_from_iter, Connection};
use serde_json::{json, Map, Value};

use std::time::Instant;

use crate::alteracoes::{self, Nova};
use crate::comandos::{self, Contexto};
use crate::error::{AppError, AppResult};
use crate::rede_regras::{decidir_promocao, Decisao, Membro, SessaoRede, Sessoes};
use crate::seguranca::sha256_hex;

/// Resposta do servidor quando falta sessão de usuário válida (dispositivo autorizado sem login).
pub const MSG_SEM_SESSAO: &str = "Faça login para ver os dados do servidor.";

/// Tabelas de dados do negócio (as que entram no snapshot). Config e registro de alterações são locais.
pub const TABELAS: &[&str] = &[
    "funcionarios", "clientes", "itens", "veiculos", "fotos_veiculos", "ordens_servico", "ordem_itens", "ordem_km", "caixas",
    "caixa_movimentos", "fidelidade_movimentos",
    "fornecedores", "compras", "compra_itens", "contas_pagar_compra",
    "orcamentos", "orcamento_itens",
    "recebimentos", "contas_receber", "contas_pagar", "pagamentos_pagar",
];

fn para_json(v: SqlValue) -> Value {
    match v {
        SqlValue::Null => Value::Null,
        SqlValue::Integer(i) => json!(i),
        SqlValue::Real(f) => serde_json::Number::from_f64(f).map(Value::Number).unwrap_or(Value::Null),
        SqlValue::Text(s) => Value::String(s),
        SqlValue::Blob(b) => Value::String(B64.encode(b)),
    }
}

fn linha(row: &rusqlite::Row, colunas: &[String]) -> rusqlite::Result<Value> {
    let mut m = Map::new();
    for (i, c) in colunas.iter().enumerate() {
        m.insert(c.clone(), para_json(row.get::<_, SqlValue>(i)?));
    }
    Ok(Value::Object(m))
}

/// Linha de uma tabela como JSON (ou None se não existir).
pub fn linha_json(conn: &Connection, tabela: &str, id: i64) -> AppResult<Option<Value>> {
    checar_tabela(tabela)?;
    let mut stmt = conn.prepare(&format!("SELECT * FROM {tabela} WHERE id = ?1"))?;
    let colunas: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
    let mut rows = stmt.query([id])?;
    match rows.next()? {
        Some(r) => Ok(Some(linha(r, &colunas)?)),
        None => Ok(None),
    }
}

/// Impressão digital de uma linha: usada para detectar se o registro mudou no servidor.
pub fn hash_linha(v: &Value) -> String {
    sha256_hex(&v.to_string())
}

/// Snapshot completo das tabelas de dados.
pub fn snapshot(conn: &Connection) -> AppResult<Value> {
    let mut out = Map::new();
    for t in TABELAS {
        let mut stmt = conn.prepare(&format!("SELECT * FROM {t} ORDER BY rowid"))?;
        let colunas: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
        let rows = stmt.query_map([], |r| linha(r, &colunas))?.collect::<Result<Vec<_>, _>>()?;
        out.insert(t.to_string(), Value::Array(rows));
    }
    Ok(Value::Object(out))
}

/// Snapshot para exportar a um arquivo: igual ao `snapshot`, mas sem o hash de senha dos funcionários.
/// (Config, tokens e chave da rede nunca entram no snapshot, pois não são tabelas de dados.)
pub fn snapshot_exportacao(conn: &Connection) -> AppResult<Value> {
    let mut dados = snapshot(conn)?;
    if let Some(linhas) = dados.get_mut("funcionarios").and_then(Value::as_array_mut) {
        for linha in linhas.iter_mut() {
            if let Some(m) = linha.as_object_mut() {
                m.remove("senha_hash");
            }
        }
    }
    Ok(dados)
}

/// Substitui o conteúdo das tabelas de dados pelo snapshot recebido (dentro de uma transação).
pub fn aplicar_snapshot(conn: &mut Connection, dados: &Value) -> AppResult<()> {
    let tx = conn.transaction()?;
    substituir_dados(&tx, dados)?;
    tx.commit()?;
    Ok(())
}

/// Substitui a cópia local pelo snapshot e, na mesma transação, refaz por cima as alterações que o servidor
/// ainda não aceitou (pendente, conflito, erro). Sem isso, essas alterações sumiriam da tela.
/// Devolve quantas alterações da fila foram refeitas.
pub fn aplicar_snapshot_preservando_fila(conn: &mut Connection, dados: &Value) -> AppResult<usize> {
    let tx = conn.transaction()?;
    substituir_dados(&tx, dados)?;
    let refeitas = crate::alteracoes::reaplicar_sobre(&tx)?;
    tx.commit()?;
    Ok(refeitas)
}

fn substituir_dados(conn: &Connection, dados: &Value) -> AppResult<()> {
    let obj = dados.as_object().ok_or_else(|| AppError::Database("snapshot inválido".into()))?;
    for t in TABELAS.iter().rev() {
        conn.execute(&format!("DELETE FROM {t}"), [])?;
    }
    for t in TABELAS {
        let Some(linhas) = obj.get(*t).and_then(Value::as_array) else { continue };
        for l in linhas {
            let m = l.as_object().ok_or_else(|| AppError::Database("linha inválida".into()))?;
            let colunas: Vec<&String> = m.keys().collect();
            let mut valores = Vec::with_capacity(colunas.len());
            for c in &colunas {
                let v = &m[*c];
                valores.push(match v {
                    Value::Null => SqlValue::Null,
                    Value::Number(n) if n.is_i64() => SqlValue::Integer(n.as_i64().unwrap()),
                    Value::Number(n) => SqlValue::Real(n.as_f64().unwrap_or(0.0)),
                    Value::String(s) if *t == "fotos_veiculos" && c.as_str() == "dados" => {
                        SqlValue::Blob(B64.decode(s).map_err(|_| AppError::Database("foto inválida".into()))?)
                    }
                    Value::String(s) => SqlValue::Text(s.clone()),
                    outro => SqlValue::Text(outro.to_string()),
                });
            }
            let nomes: Vec<&str> = colunas.iter().map(|c| c.as_str()).collect();
            let marcas: Vec<String> = (1..=nomes.len()).map(|i| format!("?{i}")).collect();
            conn.execute(
                &format!("INSERT INTO {t} ({}) VALUES ({})", nomes.join(", "), marcas.join(", ")),
                params_from_iter(valores),
            )?;
        }
    }
    Ok(())
}

pub fn checar_tabela(t: &str) -> AppResult<()> {
    if TABELAS.contains(&t) {
        Ok(())
    } else {
        Err(AppError::Database(format!("tabela desconhecida: {t}")))
    }
}

// ---- Servidor: acesso à cópia dos dados ----------------------------------------------------

/// Leitura da cópia dos dados e escritas pela rede: só com token de sessão de usuário emitido por este servidor
/// para este mesmo dispositivo. Dispositivo autorizado da licença sem usuário logado não recebe nada.
pub fn exigir_sessao(sessoes: &Sessoes, token: &str, dispositivo: &str, agora: Instant) -> AppResult<SessaoRede> {
    sessoes.validar(token, dispositivo, agora).ok_or_else(|| AppError::Conflict(MSG_SEM_SESSAO.into()))
}

/// Uma escrita recebida pela rede. `uuid` é o identificador único da alteração no cliente (ver `alteracoes`).
pub struct Escrita<'a> {
    pub uuid: Option<&'a str>,
    pub nome: &'a str,
    pub args: &'a Value,
    pub base: Option<&'a str>,
    pub ctx: &'a Contexto,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Resposta {
    /// Aplicada agora; traz o valor do comando.
    Aplicada(Value),
    /// Já tinha sido aplicada antes (o cliente reenviou sem saber se o servidor recebeu): não se aplica de novo.
    Repetida(Value),
    /// O registro mudou no servidor desde a base do cliente: traz a versão atual.
    Conflito(Value),
}

/// Aplica uma escrita vinda da rede, uma única vez por identificador. Quem chama já checou a sessão.
pub fn processar_escrita(conn: &rusqlite::Connection, e: &Escrita) -> AppResult<Resposta> {
    if let Some(uuid) = e.uuid {
        if let Some(resultado) = alteracoes::resultado_aplicado(conn, uuid)? {
            return Ok(Resposta::Repetida(resultado));
        }
    }
    if comandos::e_escrita(e.nome) {
        if let Some(base) = e.base {
            if let Some(atual) = comandos::verificar_base(conn, e.nome, e.args, base)? {
                return Ok(Resposta::Conflito(atual));
            }
        }
    }
    let valor = comandos::executar(conn, e.nome, e.args, e.ctx)?;
    if comandos::e_escrita(e.nome) {
        let n = Nova {
            computador: &e.ctx.computador,
            usuario: &e.ctx.usuario,
            comando: e.nome,
            args: e.args,
            tabela: comandos::tabela_do_comando(e.nome),
            id_local: None,
            base_hash: None,
            status: "sincronizado",
        };
        match e.uuid {
            Some(uuid) => alteracoes::registrar_aplicada(conn, uuid, &n, &valor)?,
            None => {
                alteracoes::registrar(conn, &n)?;
            }
        }
    }
    Ok(Resposta::Aplicada(valor))
}

// ---- Eleição: situação do servidor -----------------------------------------------------------

/// Situação da rede quanto ao servidor, decidida a partir dos computadores que respondem agora.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SituacaoServidor {
    /// Outro computador responde como servidor.
    ServidorAtivo { dispositivo: String },
    /// Não há servidor e a eleição escolheu este computador.
    Promover,
    /// Não há condições seguras para escolher servidor. A tela deve mostrar o aviso com o motivo.
    SemServidorSeguro { motivo: String },
}

/// Eleição (função pura). `total_conhecidos` é quantos computadores da licença já foram vistos.
pub fn situacao_eleicao(eu: &Membro, outros: &[Membro], total_conhecidos: usize) -> SituacaoServidor {
    if let Some(s) = outros.iter().find(|m| m.papel == "servidor" && m.dispositivo != eu.dispositivo) {
        return SituacaoServidor::ServidorAtivo { dispositivo: s.dispositivo.clone() };
    }
    match decidir_promocao(eu, outros, total_conhecidos) {
        Decisao::Promover => SituacaoServidor::Promover,
        Decisao::Aguardar(motivo) => SituacaoServidor::SemServidorSeguro { motivo: motivo.into() },
        _ => SituacaoServidor::SemServidorSeguro { motivo: "sem decisão segura".into() },
    }
}

// ---- Rede: início e fim do ciclo ------------------------------------------------------------

/// Controle do ciclo de rede (escuta e sincronização). Cada início recebe uma geração nova;
/// ao parar, a geração muda e a thread antiga percebe que não é mais a vigente e encerra.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CicloRede {
    ligado: bool,
    geracao: u64,
}

impl CicloRede {
    pub const fn novo() -> CicloRede {
        CicloRede { ligado: false, geracao: 0 }
    }

    /// Liga o ciclo. Devolve a geração da nova execução, ou None se já estava ligado.
    pub fn iniciar(&mut self) -> Option<u64> {
        if self.ligado {
            return None;
        }
        self.ligado = true;
        self.geracao += 1;
        Some(self.geracao)
    }

    /// Desliga o ciclo. Qualquer execução anterior deixa de ser vigente.
    pub fn parar(&mut self) {
        self.ligado = false;
        self.geracao += 1;
    }

    pub fn vigente(&self, geracao: u64) -> bool {
        self.ligado && self.geracao == geracao
    }

    pub fn ligado(&self) -> bool {
        self.ligado
    }
}
