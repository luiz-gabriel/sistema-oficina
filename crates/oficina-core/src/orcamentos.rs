//! Orçamentos: proposta de serviço para o cliente aprovar antes de virar ordem de serviço.
//!
//! - Valores em CENTAVOS (inteiros). Ao aprovar, os itens ficam congelados na tabela `orcamento_itens`.
//! - Orçamento APROVADO vira ordem de serviço (`converter_os`), ligado por `orcamentos.ordem_id` e `ordens_servico.orcamento_id`.
//! - Aditivo (`ADITIVO`): peça ou serviço extra durante o serviço. Só entra na OS depois de aprovado.
//! - A aprovação é REGISTRADA pela oficina (forma informada + observação). Não é um documento que o cliente assina.
use chrono::{NaiveDate, Utc};
use rusqlite::{params, Connection, Row};
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult, FieldError};
use crate::ordens::{self, LinhaInput, Ordem};
use crate::validation::Validator;
use crate::{clientes, funcionarios, itens, veiculos};

/// Formas de aprovação aceitas. "outro" exige observação.
pub const FORMAS_APROVACAO: &[&str] = &["presencial", "telefone", "WhatsApp", "outro"];
pub const TIPO_ORCAMENTO: &str = "ORCAMENTO";
pub const TIPO_ADITIVO: &str = "ADITIVO";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Status {
    Rascunho,
    AguardandoAprovacao,
    Aprovado,
    Recusado,
    Cancelado,
}

impl Status {
    pub fn como_texto(self) -> &'static str {
        match self {
            Status::Rascunho => "RASCUNHO",
            Status::AguardandoAprovacao => "AGUARDANDO_APROVACAO",
            Status::Aprovado => "APROVADO",
            Status::Recusado => "RECUSADO",
            Status::Cancelado => "CANCELADO",
        }
    }

    pub fn de_texto(s: &str) -> Option<Status> {
        match s {
            "RASCUNHO" => Some(Status::Rascunho),
            "AGUARDANDO_APROVACAO" => Some(Status::AguardandoAprovacao),
            "APROVADO" => Some(Status::Aprovado),
            "RECUSADO" => Some(Status::Recusado),
            "CANCELADO" => Some(Status::Cancelado),
            _ => None,
        }
    }

    /// Nome que aparece para a pessoa na tela e no cupom.
    pub fn rotulo(self) -> &'static str {
        match self {
            Status::Rascunho => "Rascunho (ainda não enviado)",
            Status::AguardandoAprovacao => "Aguardando aprovação do cliente",
            Status::Aprovado => "Aprovado",
            Status::Recusado => "Recusado pelo cliente",
            Status::Cancelado => "Cancelado",
        }
    }

    /// Máquina de estados. APROVADO, RECUSADO e CANCELADO são finais.
    pub fn pode_transitar_para(self, para: Status) -> bool {
        matches!(
            (self, para),
            (Status::Rascunho, Status::AguardandoAprovacao)
                | (Status::Rascunho, Status::Cancelado)
                | (Status::AguardandoAprovacao, Status::Aprovado)
                | (Status::AguardandoAprovacao, Status::Recusado)
                | (Status::AguardandoAprovacao, Status::Cancelado)
        )
    }
}

pub fn validar_transicao(de: Status, para: Status) -> AppResult<()> {
    if de.pode_transitar_para(para) {
        Ok(())
    } else {
        Err(AppError::Conflict(format!(
            "Este orçamento está como \"{}\" e não pode passar para \"{}\".",
            de.rotulo(),
            para.rotulo()
        )))
    }
}

/// Preço em reais (como digitado no cadastro de itens) para centavos inteiros.
pub fn centavos(reais: f64) -> i64 {
    (reais * 100.0).round() as i64
}

pub fn subtotal_centavos(quantidade: i64, preco_unitario_centavos: i64) -> i64 {
    quantidade * preco_unitario_centavos
}

pub fn total_centavos(subtotais: &[i64]) -> i64 {
    subtotais.iter().sum()
}

/// Valida a forma informada na aprovação. Retorna (forma, observação limpa).
pub fn validar_aprovacao(forma: &str, observacao: Option<&str>) -> AppResult<(String, Option<String>)> {
    let obs = observacao.map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    if !FORMAS_APROVACAO.contains(&forma) {
        return Err(AppError::Validation(vec![FieldError {
            field: "forma".into(),
            message: "Escolha como o cliente aprovou: presencial, telefone, WhatsApp ou outro.".into(),
        }]));
    }
    if forma == "outro" && obs.is_none() {
        return Err(AppError::Validation(vec![FieldError {
            field: "observacao".into(),
            message: "Escreva como o cliente aprovou.".into(),
        }]));
    }
    if obs.as_deref().map(|o| o.chars().count() > 500).unwrap_or(false) {
        return Err(AppError::Validation(vec![FieldError {
            field: "observacao".into(),
            message: "máximo de 500 caracteres".into(),
        }]));
    }
    Ok((forma.to_string(), obs))
}

/// Data de validade no formato AAAA-MM-DD (o que o campo de data da tela envia).
pub fn data_valida(texto: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(texto.trim(), "%Y-%m-%d").ok()
}

/// Um orçamento vence no fim do dia da validade. Sem validade (aditivo) não vence.
pub fn dentro_da_validade(validade: Option<&str>, hoje: NaiveDate) -> bool {
    match validade.and_then(data_valida) {
        Some(limite) => hoje <= limite,
        None => true,
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct LinhaOrcamento {
    pub item_id: i64,
    pub nome: String,
    pub tipo: String,
    pub quantidade: i64,
    pub preco_unitario_centavos: i64,
    pub subtotal_centavos: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Orcamento {
    pub id: i64,
    /// "ORCAMENTO" (proposta nova) ou "ADITIVO" (peça/serviço extra de uma OS).
    pub tipo: String,
    pub cliente_id: i64,
    pub cliente_nome: String,
    pub veiculo_id: i64,
    pub veiculo_placa: String,
    pub veiculo_modelo: String,
    pub funcionario_id: i64,
    pub funcionario_nome: String,
    /// Em aditivo: a OS à qual o aditivo se refere. Em orçamento: a OS criada a partir dele (após converter).
    pub ordem_id: Option<i64>,
    pub status: Status,
    pub observacoes: Option<String>,
    /// Data limite (AAAA-MM-DD). Aditivo não tem validade.
    pub validade: Option<String>,
    /// Prazo estimado do serviço, em dias.
    pub prazo_dias: Option<i64>,
    pub criado_em: String,
    pub enviado_em: Option<String>,
    pub aprovado_em: Option<String>,
    pub aprovacao_forma: Option<String>,
    pub aprovacao_obs: Option<String>,
    pub recusado_em: Option<String>,
    pub cancelado_em: Option<String>,
    pub valor_total_centavos: i64,
    pub itens: Vec<LinhaOrcamento>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct OrcamentoInput {
    pub cliente_id: Option<i64>,
    pub veiculo_id: Option<i64>,
    pub funcionario_id: Option<i64>,
    pub observacoes: Option<String>,
    /// Data limite AAAA-MM-DD.
    pub validade: Option<String>,
    pub prazo_dias: Option<i64>,
    #[serde(default)]
    pub itens: Vec<LinhaInput>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct AditivoInput {
    pub os_id: Option<i64>,
    pub observacoes: Option<String>,
    #[serde(default)]
    pub itens: Vec<LinhaInput>,
}

/// Linha validada, com preço congelado no momento da gravação.
struct LinhaPronta {
    item_id: i64,
    nome: String,
    tipo: String,
    quantidade: i64,
    preco_unitario_centavos: i64,
    subtotal_centavos: i64,
}

fn validar_linhas(conn: &Connection, linhas: &[LinhaInput]) -> AppResult<Vec<LinhaPronta>> {
    let mut prontas: Vec<LinhaPronta> = Vec::new();
    let mut erros = Vec::new();
    for (i, linha) in linhas.iter().enumerate() {
        let mut v = Validator::new();
        let item_id = v.int("item_id", linha.item_id, true, 1, i64::MAX);
        let quantidade = v.int("quantidade", linha.quantidade, true, 1, 100_000);
        if let Err(AppError::Validation(mut e)) = v.finish() {
            for err in e.iter_mut() {
                err.field = format!("itens[{i}].{}", err.field);
            }
            erros.extend(e);
            continue;
        }
        let (item_id, quantidade) = (item_id.unwrap(), quantidade.unwrap());
        if prontas.iter().any(|p| p.item_id == item_id) {
            erros.push(FieldError {
                field: format!("itens[{i}].item_id"),
                message: "item repetido; use uma única linha com a quantidade total".into(),
            });
            continue;
        }
        let item = itens::buscar(conn, item_id)
            .map_err(|_| AppError::NotFound(format!("Item {item_id} não encontrado.")))?;
        let unitario = centavos(item.preco_venda);
        prontas.push(LinhaPronta {
            item_id,
            nome: item.nome,
            tipo: item.tipo,
            quantidade,
            preco_unitario_centavos: unitario,
            subtotal_centavos: subtotal_centavos(quantidade, unitario),
        });
    }
    if prontas.is_empty() && erros.is_empty() {
        erros.push(FieldError { field: "itens".into(), message: "Inclua ao menos uma peça ou serviço.".into() });
    }
    if erros.is_empty() {
        Ok(prontas)
    } else {
        Err(AppError::Validation(erros))
    }
}

/// Executa `f` dentro de uma transação: COMMIT se deu certo, ROLLBACK se não.
fn em_transacao<T>(conn: &Connection, f: impl FnOnce() -> AppResult<T>) -> AppResult<T> {
    conn.execute_batch("BEGIN")?;
    match f() {
        Ok(v) => {
            conn.execute_batch("COMMIT")?;
            Ok(v)
        }
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}

fn gravar_itens(conn: &Connection, orcamento_id: i64, linhas: &[LinhaPronta]) -> AppResult<()> {
    for l in linhas {
        conn.execute(
            "INSERT INTO orcamento_itens (orcamento_id, item_id, nome, tipo, quantidade, preco_unitario_centavos, subtotal_centavos)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![orcamento_id, l.item_id, l.nome, l.tipo, l.quantidade, l.preco_unitario_centavos, l.subtotal_centavos],
        )?;
    }
    Ok(())
}

const SELECT_BASE: &str = "
    SELECT o.id, o.tipo, o.cliente_id, COALESCE(c.nome, ''), o.veiculo_id, COALESCE(v.placa, ''), COALESCE(v.modelo, ''),
           o.funcionario_id, COALESCE(f.nome, ''), o.ordem_id, o.status, o.observacoes, o.validade, o.prazo_dias,
           o.criado_em, o.enviado_em, o.aprovado_em, o.aprovacao_forma, o.aprovacao_obs, o.recusado_em, o.cancelado_em,
           o.valor_total_centavos
    FROM orcamentos o
    LEFT JOIN clientes c     ON c.id = o.cliente_id
    LEFT JOIN veiculos v     ON v.id = o.veiculo_id
    LEFT JOIN funcionarios f ON f.id = o.funcionario_id";

fn de_linha(r: &Row) -> rusqlite::Result<Orcamento> {
    let status_txt: String = r.get(10)?;
    Ok(Orcamento {
        id: r.get(0)?,
        tipo: r.get(1)?,
        cliente_id: r.get(2)?,
        cliente_nome: r.get(3)?,
        veiculo_id: r.get(4)?,
        veiculo_placa: r.get(5)?,
        veiculo_modelo: r.get(6)?,
        funcionario_id: r.get(7)?,
        funcionario_nome: r.get(8)?,
        ordem_id: r.get(9)?,
        status: Status::de_texto(&status_txt).unwrap_or(Status::Rascunho),
        observacoes: r.get(11)?,
        validade: r.get(12)?,
        prazo_dias: r.get(13)?,
        criado_em: r.get(14)?,
        enviado_em: r.get(15)?,
        aprovado_em: r.get(16)?,
        aprovacao_forma: r.get(17)?,
        aprovacao_obs: r.get(18)?,
        recusado_em: r.get(19)?,
        cancelado_em: r.get(20)?,
        valor_total_centavos: r.get(21)?,
        itens: Vec::new(),
    })
}

fn carregar_itens(conn: &Connection, orc: &mut Orcamento) -> AppResult<()> {
    let mut stmt = conn.prepare(
        "SELECT item_id, nome, tipo, quantidade, preco_unitario_centavos, subtotal_centavos
         FROM orcamento_itens WHERE orcamento_id = ?1 ORDER BY id",
    )?;
    orc.itens = stmt
        .query_map([orc.id], |r| {
            Ok(LinhaOrcamento {
                item_id: r.get(0)?,
                nome: r.get(1)?,
                tipo: r.get(2)?,
                quantidade: r.get(3)?,
                preco_unitario_centavos: r.get(4)?,
                subtotal_centavos: r.get(5)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    Ok(())
}

pub fn buscar(conn: &Connection, id: i64) -> AppResult<Orcamento> {
    let mut orc = conn
        .query_row(&format!("{SELECT_BASE} WHERE o.id = ?1"), [id], de_linha)
        .map_err(|_| AppError::NotFound("Orçamento não encontrado.".into()))?;
    carregar_itens(conn, &mut orc)?;
    Ok(orc)
}

pub fn listar(conn: &Connection) -> AppResult<Vec<Orcamento>> {
    let mut stmt = conn.prepare(&format!("{SELECT_BASE} ORDER BY o.id DESC"))?;
    let mut lista: Vec<Orcamento> = stmt.query_map([], de_linha)?.collect::<Result<_, _>>()?;
    for orc in lista.iter_mut() {
        carregar_itens(conn, orc)?;
    }
    Ok(lista)
}

fn status_atual(conn: &Connection, id: i64) -> AppResult<(String, Status)> {
    let (tipo, st): (String, String) = conn
        .query_row("SELECT tipo, status FROM orcamentos WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(|_| AppError::NotFound("Orçamento não encontrado.".into()))?;
    let status = Status::de_texto(&st).unwrap_or(Status::Rascunho);
    Ok((tipo, status))
}

fn mudar_status(conn: &Connection, id: i64, para: Status, agora: &str) -> AppResult<()> {
    let (_, de) = status_atual(conn, id)?;
    validar_transicao(de, para)?;
    let coluna_data = match para {
        Status::AguardandoAprovacao => "enviado_em",
        Status::Recusado => "recusado_em",
        Status::Cancelado => "cancelado_em",
        _ => "criado_em",
    };
    conn.execute(
        &format!("UPDATE orcamentos SET status = ?1, {coluna_data} = ?2 WHERE id = ?3"),
        params![para.como_texto(), agora, id],
    )?;
    Ok(())
}

/// Cria um orçamento em RASCUNHO. Cliente e veículo precisam bater (o veículo é do cliente).
pub fn criar(conn: &Connection, input: &OrcamentoInput, agora: &str) -> AppResult<Orcamento> {
    let mut v = Validator::new();
    let cliente_id = v.int("cliente_id", input.cliente_id, true, 1, i64::MAX);
    let veiculo_id = v.int("veiculo_id", input.veiculo_id, true, 1, i64::MAX);
    let funcionario_id = v.int("funcionario_id", input.funcionario_id, true, 1, i64::MAX);
    let observacoes = v.text("observacoes", input.observacoes.as_deref(), false, 1000);
    let prazo = match input.prazo_dias {
        Some(d) if !(0..=3650).contains(&d) => {
            v.fail("prazo_dias", "deve estar entre 0 e 3650 dias");
            None
        }
        outro => outro,
    };
    let validade_txt = input.validade.clone().unwrap_or_default();
    match data_valida(&validade_txt) {
        None => v.fail("validade", "Informe a data limite do orçamento."),
        Some(data) if data < Utc::now().date_naive() => v.fail("validade", "A data limite já passou. Escolha uma data de hoje em diante."),
        Some(_) => {}
    }
    v.finish()?;
    let (cliente_id, veiculo_id, funcionario_id) = (cliente_id.unwrap(), veiculo_id.unwrap(), funcionario_id.unwrap());

    clientes::buscar(conn, cliente_id)?;
    let veiculo = veiculos::buscar(conn, veiculo_id)?;
    if veiculo.cliente_id != cliente_id {
        return Err(AppError::Validation(vec![FieldError {
            field: "veiculo_id".into(),
            message: "Este veículo não está cadastrado para este cliente.".into(),
        }]));
    }
    funcionarios::buscar(conn, funcionario_id)?;
    let linhas = validar_linhas(conn, &input.itens)?;
    let total = total_centavos(&linhas.iter().map(|l| l.subtotal_centavos).collect::<Vec<_>>());

    let id = em_transacao(conn, || {
        conn.execute(
            "INSERT INTO orcamentos (tipo, cliente_id, veiculo_id, funcionario_id, status, observacoes, validade,
                                     prazo_dias, criado_em, valor_total_centavos)
             VALUES ('ORCAMENTO', ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![cliente_id, veiculo_id, funcionario_id, Status::Rascunho.como_texto(), observacoes,
                    validade_txt.trim(), prazo, agora, total],
        )?;
        let id = conn.last_insert_rowid();
        gravar_itens(conn, id, &linhas)?;
        Ok(id)
    })?;
    buscar(conn, id)
}

/// RASCUNHO -> AGUARDANDO_APROVACAO. Depois disso o orçamento fica parado para o cliente decidir.
pub fn enviar(conn: &Connection, id: i64, agora: &str) -> AppResult<Orcamento> {
    mudar_status(conn, id, Status::AguardandoAprovacao, agora)?;
    buscar(conn, id)
}

/// AGUARDANDO_APROVACAO -> APROVADO. Registra a forma informada pelo cliente.
/// Aditivo aprovado entra na OS (itens copiados e total da OS recalculado).
/// Orçamento não é recalculado: os valores são os que foram congelados na criação.
pub fn aprovar(conn: &Connection, id: i64, forma: &str, observacao: Option<&str>, agora: &str) -> AppResult<Orcamento> {
    let (tipo, de) = status_atual(conn, id)?;
    validar_transicao(de, Status::Aprovado)?;
    let (forma, obs) = validar_aprovacao(forma, observacao)?;
    let orc = buscar(conn, id)?;
    if !dentro_da_validade(orc.validade.as_deref(), Utc::now().date_naive()) {
        let venceu = orc.validade.as_deref().and_then(data_valida).map(|d| d.format("%d/%m/%Y").to_string()).unwrap_or_default();
        return Err(AppError::Conflict(format!(
            "Este orçamento venceu em {venceu}. Faça um novo orçamento para o cliente."
        )));
    }
    if tipo == TIPO_ADITIVO {
        let os_id = orc.ordem_id.ok_or_else(|| AppError::Conflict("Adicional sem ordem de serviço.".into()))?;
        let st_os: String = conn.query_row("SELECT status FROM ordens_servico WHERE id = ?1", [os_id], |r| r.get(0))?;
        if !matches!(st_os.as_str(), "CRIADA" | "EM_ANDAMENTO") {
            return Err(AppError::Conflict(
                "Não é possível aprovar: a ordem de serviço já foi concluída ou cancelada.".into(),
            ));
        }
    }

    em_transacao(conn, || {
        conn.execute(
            "UPDATE orcamentos SET status = ?1, aprovado_em = ?2, aprovacao_forma = ?3, aprovacao_obs = ?4 WHERE id = ?5",
            params![Status::Aprovado.como_texto(), agora, forma, obs, id],
        )?;
        if let (true, Some(os_id)) = (tipo == TIPO_ADITIVO, orc.ordem_id) {
            for l in &orc.itens {
                let unitario = l.preco_unitario_centavos as f64 / 100.0;
                conn.execute(
                    "INSERT INTO ordem_itens (os_id, item_id, quantidade, preco_unitario, subtotal) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![os_id, l.item_id, l.quantidade, unitario, l.subtotal_centavos as f64 / 100.0],
                )?;
            }
            conn.execute(
                "UPDATE ordens_servico SET valor_total = ROUND((SELECT COALESCE(SUM(subtotal), 0) FROM ordem_itens WHERE os_id = ?1), 2)
                 WHERE id = ?1",
                [os_id],
            )?;
        }
        Ok(())
    })?;
    buscar(conn, id)
}

pub fn recusar(conn: &Connection, id: i64, agora: &str) -> AppResult<Orcamento> {
    mudar_status(conn, id, Status::Recusado, agora)?;
    buscar(conn, id)
}

pub fn cancelar(conn: &Connection, id: i64, agora: &str) -> AppResult<Orcamento> {
    mudar_status(conn, id, Status::Cancelado, agora)?;
    buscar(conn, id)
}

/// Cria a ordem de serviço a partir de um orçamento APROVADO, sem redigitar cliente, veículo ou itens.
/// O orçamento continua guardado (não é apagado nem recalculado) e fica ligado à OS nos dois sentidos.
pub fn converter_os(conn: &Connection, id: i64, funcionario_id: Option<i64>, agora: &str) -> AppResult<Ordem> {
    let orc = buscar(conn, id)?;
    if orc.tipo != TIPO_ORCAMENTO {
        return Err(AppError::Conflict("Só é possível transformar um orçamento em ordem de serviço.".into()));
    }
    if orc.status != Status::Aprovado {
        return Err(AppError::Conflict(
            "Só é possível transformar em ordem de serviço um orçamento aprovado pelo cliente.".into(),
        ));
    }
    if orc.ordem_id.is_some() {
        return Err(AppError::Conflict("Este orçamento já foi transformado em ordem de serviço.".into()));
    }
    let responsavel = funcionario_id.unwrap_or(orc.funcionario_id);
    funcionarios::buscar(conn, responsavel)?;

    let os_id = em_transacao(conn, || {
        conn.execute(
            "INSERT INTO ordens_servico (tipo, veiculo_id, funcionario_id, descricao, status, data_entrada, valor_total, orcamento_id)
             VALUES ('OS', ?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                orc.veiculo_id,
                responsavel,
                format!("Criada a partir do orçamento nº {id}."),
                crate::ordens::rules::Status::Criada.como_texto(),
                agora,
                orc.valor_total_centavos as f64 / 100.0,
                id,
            ],
        )?;
        let os_id = conn.last_insert_rowid();
        for l in &orc.itens {
            conn.execute(
                "INSERT INTO ordem_itens (os_id, item_id, quantidade, preco_unitario, subtotal) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![os_id, l.item_id, l.quantidade, l.preco_unitario_centavos as f64 / 100.0, l.subtotal_centavos as f64 / 100.0],
            )?;
        }
        conn.execute("UPDATE orcamentos SET ordem_id = ?1 WHERE id = ?2", params![os_id, id])?;
        Ok(os_id)
    })?;
    ordens::buscar(conn, os_id)
}

/// Peça ou serviço extra numa OS em andamento. Vira um orçamento ADITIVO em RASCUNHO:
/// só entra na OS depois de enviado e aprovado.
pub fn adicional(conn: &Connection, input: &AditivoInput, agora: &str) -> AppResult<Orcamento> {
    let mut v = Validator::new();
    let os_id = v.int("os_id", input.os_id, true, 1, i64::MAX);
    v.finish()?;
    let os_id = os_id.unwrap();
    let (tipo_os, status_os, veiculo_id, funcionario_id): (String, String, Option<i64>, i64) = conn
        .query_row(
            "SELECT tipo, status, veiculo_id, funcionario_id FROM ordens_servico WHERE id = ?1",
            [os_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .map_err(|_| AppError::NotFound("Ordem de serviço não encontrada.".into()))?;
    if tipo_os != "OS" || !matches!(status_os.as_str(), "CRIADA" | "EM_ANDAMENTO") {
        return Err(AppError::Conflict(
            "Só é possível adicionar peça ou serviço em uma ordem criada ou em andamento.".into(),
        ));
    }
    let veiculo_id = veiculo_id.ok_or_else(|| AppError::Conflict("Ordem sem veículo.".into()))?;
    let cliente_id = veiculos::buscar(conn, veiculo_id)?.cliente_id;
    let linhas = validar_linhas(conn, &input.itens)?;
    let total = total_centavos(&linhas.iter().map(|l| l.subtotal_centavos).collect::<Vec<_>>());
    let observacoes = input.observacoes.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);

    let id = em_transacao(conn, || {
        conn.execute(
            "INSERT INTO orcamentos (tipo, cliente_id, veiculo_id, funcionario_id, ordem_id, status, observacoes,
                                     criado_em, valor_total_centavos)
             VALUES ('ADITIVO', ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![cliente_id, veiculo_id, funcionario_id, os_id, Status::Rascunho.como_texto(), observacoes, agora, total],
        )?;
        let id = conn.last_insert_rowid();
        gravar_itens(conn, id, &linhas)?;
        Ok(id)
    })?;
    buscar(conn, id)
}
