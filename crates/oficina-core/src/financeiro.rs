//! Financeiro: recebimentos (sinal, parciais e divididos), contas a receber, contas a pagar e resumo.
//!
//! Regras principais:
//! - Todo valor é inteiro, em centavos.
//! - Recebimento não muda o status da OS. A situação de pagamento (NAO_PAGA, PARCIALMENTE_PAGA, PAGA)
//!   é calculada a partir dos recebimentos.
//! - Cada recebimento grava o `caixa_id` do caixa aberto (se houver). O caixa soma recebimentos e nunca as OS,
//!   então um valor não entra duas vezes.
//! - Cada dívida tem uma única conta a receber (UNIQUE em origem + referência).
//! - `data` é o horário local em RFC 3339. Filtros de período usam só os 10 primeiros caracteres (AAAA-MM-DD).
use chrono::{Local, NaiveDate};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;

use crate::caixa::PAGAMENTOS;
use crate::error::{AppError, AppResult, FieldError};
use crate::ordens::{self, Status};

const SITUACOES_RECEBER: &[&str] = &["ABERTA", "PARCIAL", "QUITADA", "VENCIDA"];
const SITUACOES_PAGAR: &[&str] = &["ABERTA", "PARCIAL", "PAGA", "VENCIDA"];

/// Horário local atual em RFC 3339 (usado em todos os registros financeiros).
pub fn agora() -> String {
    Local::now().to_rfc3339()
}

fn hoje() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

fn erro(campo: &str, msg: &str) -> AppError {
    AppError::Validation(vec![FieldError { field: campo.into(), message: msg.into() }])
}

/// Texto do valor em reais para mensagens: "R$ 12,50".
fn dinheiro(centavos: i64) -> String {
    format!("R$ {},{:02}", centavos / 100, (centavos % 100).abs())
}

/// Converte reais (valor de `ordens_servico.valor_total`) em centavos inteiros.
pub fn centavos_de_reais(valor: f64) -> i64 {
    (valor * 100.0).round() as i64
}

fn valor_positivo(valor: i64, campo: &str) -> AppResult<i64> {
    if valor > 0 {
        Ok(valor)
    } else {
        Err(erro(campo, "informe um valor maior que zero"))
    }
}

fn data_valida(texto: &str, campo: &str) -> AppResult<String> {
    NaiveDate::parse_from_str(texto, "%Y-%m-%d")
        .map(|_| texto.to_string())
        .map_err(|_| erro(campo, "use a data no formato ano-mês-dia, por exemplo 2026-11-30"))
}

fn data_opcional(texto: Option<&str>, campo: &str) -> AppResult<Option<String>> {
    match texto.map(str::trim).filter(|t| !t.is_empty()) {
        Some(t) => data_valida(t, campo).map(Some),
        None => Ok(None),
    }
}

fn situacao_valida(texto: Option<&str>, permitidas: &[&str]) -> AppResult<Option<String>> {
    match texto.map(str::trim).filter(|t| !t.is_empty()) {
        None => Ok(None),
        Some(t) if permitidas.contains(&t) => Ok(Some(t.to_string())),
        Some(_) => Err(erro("situacao", &format!("escolha uma destas: {}", permitidas.join(", ")))),
    }
}

/// Caixa aberto agora, se houver.
pub fn caixa_aberto_id(conn: &Connection) -> AppResult<Option<i64>> {
    Ok(conn
        .query_row("SELECT id FROM caixas WHERE fechado_em IS NULL ORDER BY id DESC LIMIT 1", [], |r| r.get(0))
        .optional()?)
}

/// Total já recebido de uma ordem, em centavos (sinal, parciais e pagamento final).
pub fn recebido_ordem(conn: &Connection, ordem_id: i64) -> AppResult<i64> {
    Ok(conn.query_row(
        "SELECT COALESCE(SUM(valor_centavos), 0) FROM recebimentos WHERE ordem_id = ?1",
        [ordem_id],
        |r| r.get(0),
    )?)
}

/// Valor da ordem em centavos.
fn total_ordem(conn: &Connection, ordem_id: i64) -> AppResult<i64> {
    let reais: f64 = conn
        .query_row("SELECT valor_total FROM ordens_servico WHERE id = ?1", [ordem_id], |r| r.get(0))
        .optional()?
        .ok_or_else(|| AppError::NotFound("Ordem de serviço não encontrada.".into()))?;
    Ok(centavos_de_reais(reais))
}

/// Quanto ainda falta receber da ordem, em centavos.
pub fn saldo_ordem(conn: &Connection, ordem_id: i64) -> AppResult<i64> {
    Ok(total_ordem(conn, ordem_id)? - recebido_ordem(conn, ordem_id)?)
}

/// Ao alterar os itens, o novo total não pode ficar abaixo do que já foi recebido.
pub fn conferir_nao_passa_total(conn: &Connection, ordem_id: i64) -> AppResult<()> {
    let recebido = recebido_ordem(conn, ordem_id)?;
    if total_ordem(conn, ordem_id)? < recebido {
        return Err(AppError::Conflict(format!(
            "Os itens ficaram abaixo do que já foi recebido ({}). Ajuste o valor antes de salvar.",
            dinheiro(recebido)
        )));
    }
    Ok(())
}

fn situacao_pagamento_receber(saldo: i64, total: i64, vencimento: &str, hoje: &str) -> &'static str {
    if saldo <= 0 {
        "QUITADA"
    } else if vencimento < hoje {
        "VENCIDA"
    } else if saldo < total {
        "PARCIAL"
    } else {
        "ABERTA"
    }
}

fn situacao_pagamento_pagar(saldo: i64, pago: i64, vencimento: &str, hoje: &str) -> &'static str {
    if saldo <= 0 {
        "PAGA"
    } else if vencimento < hoje {
        "VENCIDA"
    } else if pago > 0 {
        "PARCIAL"
    } else {
        "ABERTA"
    }
}

fn inserir_conta_receber(
    conn: &Connection,
    origem: &str,
    referencia: i64,
    cliente_id: Option<i64>,
    descricao: &str,
    total: i64,
    saldo: i64,
    vencimento: &str,
) -> AppResult<()> {
    let existe: i64 = conn.query_row(
        "SELECT COUNT(*) FROM contas_receber WHERE origem = ?1 AND referencia = ?2",
        params![origem, referencia],
        |r| r.get(0),
    )?;
    if existe > 0 {
        return Err(AppError::Conflict("Esta dívida já tem conta a receber.".into()));
    }
    conn.execute(
        "INSERT INTO contas_receber (origem, referencia, cliente_id, descricao, valor_total_centavos, saldo_centavos, vencimento, data)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![origem, referencia, cliente_id, descricao, total, saldo, vencimento, agora()],
    )?;
    Ok(())
}

/// Abre a conta a receber quando o serviço é concluído (saldo = valor da ordem menos o que já entrou de sinal).
pub fn abrir_conta_os(conn: &Connection, ordem_id: i64, cliente_id: Option<i64>, vencimento: Option<&str>) -> AppResult<()> {
    let vencimento = match vencimento {
        Some(v) => data_valida(v, "vencimento")?,
        None => hoje(),
    };
    let total = total_ordem(conn, ordem_id)?;
    let saldo = total - recebido_ordem(conn, ordem_id)?;
    inserir_conta_receber(conn, "OS", ordem_id, cliente_id, &format!("Ordem de serviço nº {ordem_id}"), total, saldo, &vencimento)
}

/// Venda de balcão: já nasce paga. Grava a conta (saldo zero) e o recebimento da forma escolhida, no caixa aberto.
pub fn registrar_venda_balcao(
    conn: &Connection,
    ordem_id: i64,
    cliente_id: Option<i64>,
    total_centavos: i64,
    forma: &str,
    usuario: &str,
) -> AppResult<()> {
    inserir_conta_receber(
        conn,
        "VENDA",
        ordem_id,
        cliente_id,
        &format!("Venda de balcão nº {ordem_id}"),
        total_centavos,
        0,
        &hoje(),
    )?;
    if total_centavos > 0 {
        let caixa = caixa_aberto_id(conn)?;
        inserir_recebimento(conn, ordem_id, total_centavos, forma, usuario, &agora(), caixa)?;
    }
    Ok(())
}

fn inserir_recebimento(
    conn: &Connection,
    ordem_id: i64,
    valor: i64,
    forma: &str,
    usuario: &str,
    data: &str,
    caixa_id: Option<i64>,
) -> AppResult<i64> {
    conn.execute(
        "INSERT INTO recebimentos (ordem_id, valor_centavos, forma, data, usuario, caixa_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![ordem_id, valor, forma, data, usuario, caixa_id],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Registra um recebimento (sinal, parcial ou parte de pagamento dividido) de uma ordem.
/// Não muda o status da OS. Atualiza a conta a receber (se já existir) e entra no caixa aberto.
/// Não chama BEGIN: quem chama controla a transação.
pub fn registrar_recebimento_tx(
    conn: &Connection,
    ordem_id: i64,
    valor_centavos: i64,
    forma: &str,
    usuario: &str,
    data: &str,
) -> AppResult<Recebimento> {
    let valor = valor_positivo(valor_centavos, "valor_centavos")?;
    if !PAGAMENTOS.contains(&forma) {
        return Err(erro("forma", "Escolha dinheiro, pix, cartão ou outro."));
    }
    let os = ordens::buscar(conn, ordem_id)?;
    if os.status == Status::Cancelada {
        return Err(AppError::Conflict("Ordem cancelada não recebe pagamento.".into()));
    }
    let saldo = saldo_ordem(conn, ordem_id)?;
    if saldo <= 0 {
        return Err(AppError::Conflict("Esta ordem já está paga.".into()));
    }
    if valor > saldo {
        return Err(erro("valor_centavos", &format!("o valor passa do que falta receber ({})", dinheiro(saldo))));
    }
    // Mesma ordem, mesmo valor, mesma hora: provável duplicidade (toque duplo ou envio repetido).
    let repetido: i64 = conn.query_row(
        "SELECT COUNT(*) FROM recebimentos WHERE ordem_id = ?1 AND valor_centavos = ?2 AND substr(data, 1, 13) = substr(?3, 1, 13)",
        params![ordem_id, valor, data],
        |r| r.get(0),
    )?;
    if repetido > 0 {
        return Err(AppError::Conflict(
            "Este mesmo valor já foi recebido nesta ordem nesta hora. Confira a lista de recebimentos antes de registrar de novo.".into(),
        ));
    }
    let caixa = caixa_aberto_id(conn)?;
    let id = inserir_recebimento(conn, ordem_id, valor, forma, usuario, data, caixa)?;
    let origem = if os.tipo == "BALCAO" { "VENDA" } else { "OS" };
    conn.execute(
        "UPDATE contas_receber SET saldo_centavos = saldo_centavos - ?1 WHERE origem = ?2 AND referencia = ?3",
        params![valor, origem, ordem_id],
    )?;
    buscar_recebimento(conn, id)
}

#[derive(Debug, Clone, Serialize)]
pub struct Recebimento {
    pub id: i64,
    pub ordem_id: i64,
    pub valor_centavos: i64,
    pub forma: String,
    pub data: String,
    pub usuario: String,
    pub caixa_id: Option<i64>,
}

fn de_recebimento(r: &Row) -> rusqlite::Result<Recebimento> {
    Ok(Recebimento {
        id: r.get(0)?,
        ordem_id: r.get(1)?,
        valor_centavos: r.get(2)?,
        forma: r.get(3)?,
        data: r.get(4)?,
        usuario: r.get(5)?,
        caixa_id: r.get(6)?,
    })
}

const COLUNAS_RECEBIMENTO: &str = "id, ordem_id, valor_centavos, forma, data, usuario, caixa_id";

fn buscar_recebimento(conn: &Connection, id: i64) -> AppResult<Recebimento> {
    Ok(conn.query_row(&format!("SELECT {COLUNAS_RECEBIMENTO} FROM recebimentos WHERE id = ?1"), [id], de_recebimento)?)
}

/// Filtros comuns das listagens. Datas são AAAA-MM-DD, inclusivas.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct Filtro {
    pub desde: Option<String>,
    pub ate: Option<String>,
    pub situacao: Option<String>,
    pub ordem_id: Option<i64>,
}

impl Filtro {
    fn validado(mut self, permitidas: &[&str]) -> AppResult<Filtro> {
        self.desde = data_opcional(self.desde.as_deref(), "desde")?;
        self.ate = data_opcional(self.ate.as_deref(), "ate")?;
        self.situacao = situacao_valida(self.situacao.as_deref(), permitidas)?;
        Ok(self)
    }
}

pub fn recebimentos_listar(conn: &Connection, filtro: &Filtro) -> AppResult<Vec<Recebimento>> {
    let f = filtro.clone().validado(&[])?;
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUNAS_RECEBIMENTO} FROM recebimentos
         WHERE (?1 IS NULL OR ordem_id = ?1)
           AND (?2 IS NULL OR substr(data, 1, 10) >= ?2)
           AND (?3 IS NULL OR substr(data, 1, 10) <= ?3)
         ORDER BY data, id"
    ))?;
    let linhas = stmt
        .query_map(params![f.ordem_id, f.desde, f.ate], de_recebimento)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(linhas)
}

#[derive(Debug, Clone, Serialize)]
pub struct ContaReceber {
    pub id: i64,
    pub origem: String,
    pub referencia: Option<i64>,
    pub cliente_id: Option<i64>,
    pub cliente_nome: String,
    pub descricao: Option<String>,
    pub valor_total_centavos: i64,
    pub recebido_centavos: i64,
    pub saldo_centavos: i64,
    pub vencimento: String,
    pub data: String,
    pub situacao: String,
}

pub fn contas_receber_listar(conn: &Connection, filtro: &Filtro) -> AppResult<Vec<ContaReceber>> {
    let f = filtro.clone().validado(SITUACOES_RECEBER)?;
    let hoje = hoje();
    let mut stmt = conn.prepare(
        "SELECT c.id, c.origem, c.referencia, c.cliente_id, COALESCE(cl.nome, ''), c.descricao,
                c.valor_total_centavos, c.saldo_centavos, c.vencimento, c.data
         FROM contas_receber c LEFT JOIN clientes cl ON cl.id = c.cliente_id
         WHERE (?1 IS NULL OR substr(c.data, 1, 10) >= ?1) AND (?2 IS NULL OR substr(c.data, 1, 10) <= ?2)
         ORDER BY c.vencimento, c.id",
    )?;
    let mut contas = Vec::new();
    for linha in stmt.query_map(params![f.desde, f.ate], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, Option<i64>>(2)?,
            r.get::<_, Option<i64>>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, Option<String>>(5)?,
            r.get::<_, i64>(6)?,
            r.get::<_, i64>(7)?,
            r.get::<_, String>(8)?,
            r.get::<_, String>(9)?,
        ))
    })? {
        let (id, origem, referencia, cliente_id, cliente_nome, descricao, total, saldo, vencimento, data) = linha?;
        let situacao = situacao_pagamento_receber(saldo, total, &vencimento, &hoje).to_string();
        if f.situacao.as_deref().map_or(true, |s| s == situacao) {
            contas.push(ContaReceber {
                id,
                origem,
                referencia,
                cliente_id,
                cliente_nome,
                descricao,
                valor_total_centavos: total,
                recebido_centavos: total - saldo,
                saldo_centavos: saldo,
                vencimento,
                data,
                situacao,
            });
        }
    }
    Ok(contas)
}

#[derive(Debug, Clone, Deserialize)]
pub struct CadastroPagar {
    pub descricao: String,
    pub fornecedor: Option<String>,
    pub compra_id: Option<i64>,
    pub valor_centavos: i64,
    pub vencimento: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContaPagar {
    pub id: i64,
    pub descricao: String,
    pub fornecedor: Option<String>,
    pub compra_id: Option<i64>,
    pub valor_centavos: i64,
    pub pago_centavos: i64,
    pub saldo_centavos: i64,
    pub vencimento: String,
    pub data: String,
    pub situacao: String,
}

const SELECT_PAGAR: &str = "SELECT c.id, c.descricao, c.fornecedor, c.compra_id, c.valor_centavos,
       (SELECT COALESCE(SUM(p.valor_centavos), 0) FROM pagamentos_pagar p WHERE p.conta_id = c.id),
       c.vencimento, c.data
     FROM contas_pagar c";

fn de_conta_pagar(r: &Row, hoje: &str) -> rusqlite::Result<ContaPagar> {
    let valor: i64 = r.get(4)?;
    let pago: i64 = r.get(5)?;
    let vencimento: String = r.get(6)?;
    let saldo = valor - pago;
    Ok(ContaPagar {
        id: r.get(0)?,
        descricao: r.get(1)?,
        fornecedor: r.get(2)?,
        compra_id: r.get(3)?,
        valor_centavos: valor,
        pago_centavos: pago,
        saldo_centavos: saldo,
        situacao: situacao_pagamento_pagar(saldo, pago, &vencimento, hoje).to_string(),
        vencimento,
        data: r.get(7)?,
    })
}

fn buscar_conta_pagar(conn: &Connection, id: i64) -> AppResult<ContaPagar> {
    let hoje = hoje();
    conn.query_row(&format!("{SELECT_PAGAR} WHERE c.id = ?1"), [id], |r| de_conta_pagar(r, &hoje))
        .optional()?
        .ok_or_else(|| AppError::NotFound("Conta a pagar não encontrada.".into()))
}

pub fn contas_pagar_criar(conn: &Connection, dados: &CadastroPagar, usuario: &str) -> AppResult<ContaPagar> {
    let descricao = dados.descricao.trim().to_string();
    if descricao.is_empty() {
        return Err(erro("descricao", "informe o que está sendo pago"));
    }
    if descricao.chars().count() > 200 {
        return Err(erro("descricao", "use no máximo 200 letras"));
    }
    let valor = valor_positivo(dados.valor_centavos, "valor_centavos")?;
    let vencimento = data_valida(&dados.vencimento, "vencimento")?;
    let fornecedor = dados.fornecedor.as_deref().map(str::trim).filter(|f| !f.is_empty()).map(str::to_string);
    conn.execute(
        "INSERT INTO contas_pagar (descricao, fornecedor, compra_id, valor_centavos, vencimento, data, usuario)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![descricao, fornecedor, dados.compra_id, valor, vencimento, agora(), usuario],
    )?;
    buscar_conta_pagar(conn, conn.last_insert_rowid())
}

/// Registra pagamento (parcial ou total) de uma conta a pagar. `data` é AAAA-MM-DD; se omitida, usa hoje.
pub fn contas_pagar_pagar(conn: &Connection, id: i64, valor_centavos: i64, data: Option<&str>, usuario: &str) -> AppResult<ContaPagar> {
    let valor = valor_positivo(valor_centavos, "valor_centavos")?;
    let data = match data_opcional(data, "data")? {
        Some(d) => d,
        None => hoje(),
    };
    let conta = buscar_conta_pagar(conn, id)?;
    if conta.saldo_centavos <= 0 {
        return Err(AppError::Conflict("Esta conta já está paga.".into()));
    }
    if valor > conta.saldo_centavos {
        return Err(erro("valor_centavos", &format!("o valor passa do que falta pagar ({})", dinheiro(conta.saldo_centavos))));
    }
    conn.execute(
        "INSERT INTO pagamentos_pagar (conta_id, valor_centavos, data, usuario) VALUES (?1, ?2, ?3, ?4)",
        params![id, valor, data, usuario],
    )?;
    buscar_conta_pagar(conn, id)
}

pub fn contas_pagar_listar(conn: &Connection, filtro: &Filtro) -> AppResult<Vec<ContaPagar>> {
    let f = filtro.clone().validado(SITUACOES_PAGAR)?;
    let hoje = hoje();
    let mut stmt = conn.prepare(&format!(
        "{SELECT_PAGAR} WHERE (?1 IS NULL OR substr(c.data, 1, 10) >= ?1) AND (?2 IS NULL OR substr(c.data, 1, 10) <= ?2)
         ORDER BY c.vencimento, c.id"
    ))?;
    let todas = stmt
        .query_map(params![f.desde, f.ate], |r| de_conta_pagar(r, &hoje))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(todas.into_iter().filter(|c| f.situacao.as_deref().map_or(true, |s| s == c.situacao)).collect())
}

#[derive(Debug, Clone, Serialize)]
pub struct Resumo {
    pub desde: Option<String>,
    pub ate: Option<String>,
    pub situacao: Option<String>,
    /// Valor faturado: soma das contas a receber do período (valor da dívida).
    pub faturado_centavos: i64,
    /// Valor recebido: todos os recebimentos do período (inclui sinais de OS ainda não concluídas).
    pub recebido_centavos: i64,
    /// Saldo pendente a receber das contas do período.
    pub pendente_centavos: i64,
    /// Total das contas a pagar do período.
    pub a_pagar_centavos: i64,
    pub pago_centavos: i64,
    pub a_pagar_pendente_centavos: i64,
}

pub fn financeiro_resumo(conn: &Connection, filtro: &Filtro) -> AppResult<Resumo> {
    let uniao: Vec<&str> = SITUACOES_RECEBER.iter().chain(SITUACOES_PAGAR.iter()).copied().collect();
    let f = filtro.clone().validado(&uniao)?;
    // A situação vale para a lista que a tem (QUITADA só existe em receber, PAGA só em pagar).
    let situacao_receber = f.situacao.clone().filter(|s| SITUACOES_RECEBER.contains(&s.as_str()));
    let situacao_pagar = f.situacao.clone().filter(|s| SITUACOES_PAGAR.contains(&s.as_str()));
    let receber = contas_receber_listar(conn, &Filtro { situacao: situacao_receber, desde: f.desde.clone(), ate: f.ate.clone(), ..Default::default() })?;
    let pagar = contas_pagar_listar(conn, &Filtro { situacao: situacao_pagar, desde: f.desde.clone(), ate: f.ate.clone(), ..Default::default() })?;
    let recebido: i64 = conn.query_row(
        "SELECT COALESCE(SUM(valor_centavos), 0) FROM recebimentos
         WHERE (?1 IS NULL OR substr(data, 1, 10) >= ?1) AND (?2 IS NULL OR substr(data, 1, 10) <= ?2)",
        params![f.desde, f.ate],
        |r| r.get(0),
    )?;
    let a_pagar: i64 = pagar.iter().map(|c| c.valor_centavos).sum();
    let pago: i64 = pagar.iter().map(|c| c.pago_centavos).sum();
    Ok(Resumo {
        desde: f.desde.clone(),
        ate: f.ate.clone(),
        situacao: f.situacao.clone(),
        faturado_centavos: receber.iter().map(|c| c.valor_total_centavos).sum(),
        recebido_centavos: recebido,
        pendente_centavos: receber.iter().map(|c| c.saldo_centavos).sum(),
        a_pagar_centavos: a_pagar,
        pago_centavos: pago,
        a_pagar_pendente_centavos: a_pagar - pago,
    })
}

fn arg<T: DeserializeOwned>(args: &Value, chave: &str) -> AppResult<T> {
    serde_json::from_value(args.get(chave).cloned().unwrap_or(Value::Null))
        .map_err(|e| erro(chave, &format!("dados inválidos: {e}")))
}

fn ok<T: Serialize>(v: T) -> AppResult<Value> {
    serde_json::to_value(v).map_err(|e| AppError::Database(e.to_string()))
}

/// Comandos financeiros novos. Devolve None se o comando não é deste módulo.
pub fn executar(conn: &Connection, cmd: &str, args: &Value, usuario: &str) -> Option<AppResult<Value>> {
    const COMANDOS: &[&str] = &[
        "os_concluir_servico",
        "os_registrar_recebimento",
        "recebimentos_listar",
        "contas_receber_listar",
        "contas_pagar_criar",
        "contas_pagar_pagar",
        "contas_pagar_listar",
        "financeiro_resumo",
    ];
    if !COMANDOS.contains(&cmd) {
        return None;
    }
    Some(despachar(conn, cmd, args, usuario))
}

fn despachar(conn: &Connection, cmd: &str, args: &Value, usuario: &str) -> AppResult<Value> {
    let filtro = || serde_json::from_value::<Filtro>(args.clone()).map_err(|e| erro("filtro", &format!("dados inválidos: {e}")));
    match cmd {
        "os_concluir_servico" => {
            let id: i64 = arg(args, "id")?;
            let vencimento: Option<String> = arg(args, "vencimento")?;
            ok(ordens::concluir_servico(conn, id, vencimento.as_deref())?)
        }
        "os_registrar_recebimento" => {
            let ordem_id: i64 = arg(args, "ordem_id")?;
            let valor: i64 = arg(args, "valor_centavos")?;
            let forma: String = arg(args, "forma")?;
            let data = agora();
            let rec = (|| -> AppResult<Recebimento> {
                conn.execute_batch("BEGIN")?;
                match registrar_recebimento_tx(conn, ordem_id, valor, &forma, usuario, &data) {
                    Ok(r) => {
                        conn.execute_batch("COMMIT")?;
                        Ok(r)
                    }
                    Err(e) => {
                        let _ = conn.execute_batch("ROLLBACK");
                        Err(e)
                    }
                }
            })()?;
            ok(rec)
        }
        "recebimentos_listar" => ok(recebimentos_listar(conn, &filtro()?)?),
        "contas_receber_listar" => ok(contas_receber_listar(conn, &filtro()?)?),
        "contas_pagar_criar" => ok(contas_pagar_criar(conn, &arg(args, "dados")?, usuario)?),
        "contas_pagar_pagar" => {
            let id: i64 = arg(args, "id")?;
            let valor: i64 = arg(args, "valor_centavos")?;
            let data: Option<String> = arg(args, "data")?;
            ok(contas_pagar_pagar(conn, id, valor, data.as_deref(), usuario)?)
        }
        "contas_pagar_listar" => ok(contas_pagar_listar(conn, &filtro()?)?),
        "financeiro_resumo" => ok(financeiro_resumo(conn, &filtro()?)?),
        _ => Err(AppError::NotFound(format!("comando desconhecido: {cmd}"))),
    }
}
