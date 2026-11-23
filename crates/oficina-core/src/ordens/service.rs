use chrono::Utc;
use rusqlite::{params, Connection, Row};
use serde::{Deserialize, Serialize};

use super::rules::{self, Status};
use crate::error::{AppError, AppResult, FieldError};
use crate::validation::Validator;
use crate::{clientes, fidelidade, financeiro, funcionarios, veiculos};

#[derive(Debug, Clone, Serialize)]
pub struct LinhaOrdem {
    pub item_id: i64,
    pub item_nome: String,
    pub tipo: String,
    pub quantidade: i64,
    pub preco_unitario: f64,
    pub subtotal: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Ordem {
    pub id: i64,
    /// "OS" (com veículo) ou "BALCAO" (venda sem veículo).
    pub tipo: String,
    pub veiculo_id: Option<i64>,
    pub veiculo_placa: String,
    pub veiculo_modelo: String,
    pub cliente_id: Option<i64>,
    /// Vazio quando a venda de balcão não tem cliente identificado.
    pub cliente_nome: String,
    pub funcionario_id: i64,
    pub funcionario_nome: String,
    pub descricao: Option<String>,
    pub status: Status,
    pub data_entrada: String,
    pub data_conclusao: Option<String>,
    pub valor_total: f64,
    /// Forma de pagamento usada ao concluir (dinheiro, pix, cartao, outro).
    pub pagamento: Option<String>,
    pub itens: Vec<LinhaOrdem>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct LinhaInput {
    pub item_id: Option<i64>,
    pub quantidade: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct OrdemInput {
    pub veiculo_id: Option<i64>,
    pub funcionario_id: Option<i64>,
    pub descricao: Option<String>,
    #[serde(default)]
    pub itens: Vec<LinhaInput>,
}

/// Venda de balcão: peças e/ou serviços, sem veículo. Cliente é opcional. `pagamento` é obrigatório.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct BalcaoInput {
    pub cliente_id: Option<i64>,
    pub funcionario_id: Option<i64>,
    pub descricao: Option<String>,
    pub pagamento: Option<String>,
    #[serde(default)]
    pub itens: Vec<LinhaInput>,
}

/// Linha já validada e com preço congelado no momento da criação/alteração.
struct LinhaPronta {
    item_id: i64,
    quantidade: i64,
    preco_unitario: f64,
    subtotal: f64,
}

fn validar_linhas(conn: &Connection, linhas: &[LinhaInput]) -> AppResult<Vec<LinhaPronta>> {
    let mut prontas: Vec<LinhaPronta> = Vec::new();
    let mut erros = Vec::new();

    for (i, linha) in linhas.iter().enumerate() {
        let mut v = Validator::new();
        let item_id = v.int("item_id", linha.item_id, true, 1, i64::MAX);
        let quantidade = v.int("quantidade", linha.quantidade, true, 1, i64::MAX);
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

        let item = crate::itens::buscar(conn, item_id)
            .map_err(|_| AppError::NotFound(format!("Item {item_id} não encontrado.")))?;
        prontas.push(LinhaPronta {
            item_id,
            quantidade,
            preco_unitario: item.preco_venda,
            subtotal: rules::subtotal(quantidade, item.preco_venda),
        });
    }

    if erros.is_empty() {
        Ok(prontas)
    } else {
        Err(AppError::Validation(erros))
    }
}

fn gravar_linhas(conn: &Connection, os_id: i64, linhas: &[LinhaPronta]) -> AppResult<()> {
    conn.execute("DELETE FROM ordem_itens WHERE os_id = ?1", [os_id])?;
    for l in linhas {
        conn.execute(
            "INSERT INTO ordem_itens (os_id, item_id, quantidade, preco_unitario, subtotal) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![os_id, l.item_id, l.quantidade, l.preco_unitario, l.subtotal],
        )?;
    }
    let subtotais: Vec<f64> = linhas.iter().map(|l| l.subtotal).collect();
    conn.execute(
        "UPDATE ordens_servico SET valor_total = ?1 WHERE id = ?2",
        params![rules::total(&subtotais), os_id],
    )?;
    Ok(())
}

/// Cliente efetivo: o do veículo (OS) ou o próprio da venda de balcão.
const SELECT_BASE: &str = "
    SELECT os.id, os.veiculo_id, COALESCE(v.placa, ''), COALESCE(v.modelo, ''), COALESCE(c.nome, ''),
           os.funcionario_id, f.nome, os.descricao, os.status,
           os.data_entrada, os.data_conclusao, os.valor_total,
           os.tipo, COALESCE(v.cliente_id, os.cliente_id), os.pagamento
    FROM ordens_servico os
    LEFT JOIN veiculos v     ON v.id = os.veiculo_id
    LEFT JOIN clientes c     ON c.id = COALESCE(v.cliente_id, os.cliente_id)
    JOIN funcionarios f ON f.id = os.funcionario_id";

fn de_linha(r: &Row) -> rusqlite::Result<Ordem> {
    let status_txt: String = r.get(8)?;
    Ok(Ordem {
        id: r.get(0)?,
        tipo: r.get(12)?,
        veiculo_id: r.get(1)?,
        veiculo_placa: r.get(2)?,
        veiculo_modelo: r.get(3)?,
        cliente_id: r.get(13)?,
        cliente_nome: r.get(4)?,
        funcionario_id: r.get(5)?,
        funcionario_nome: r.get(6)?,
        descricao: r.get(7)?,
        status: Status::de_texto(&status_txt).unwrap_or(Status::Criada),
        data_entrada: r.get(9)?,
        data_conclusao: r.get(10)?,
        valor_total: r.get(11)?,
        pagamento: r.get(14)?,
        itens: Vec::new(),
    })
}

fn carregar_itens(conn: &Connection, os: &mut Ordem) -> AppResult<()> {
    let mut stmt = conn.prepare(
        "SELECT oi.item_id, i.nome, i.tipo, oi.quantidade, oi.preco_unitario, oi.subtotal
         FROM ordem_itens oi JOIN itens i ON i.id = oi.item_id
         WHERE oi.os_id = ?1 ORDER BY oi.id",
    )?;
    os.itens = stmt
        .query_map([os.id], |r| {
            Ok(LinhaOrdem {
                item_id: r.get(0)?,
                item_nome: r.get(1)?,
                tipo: r.get(2)?,
                quantidade: r.get(3)?,
                preco_unitario: r.get(4)?,
                subtotal: r.get(5)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    Ok(())
}

pub fn buscar(conn: &Connection, id: i64) -> AppResult<Ordem> {
    let mut os = conn
        .query_row(&format!("{SELECT_BASE} WHERE os.id = ?1"), [id], de_linha)
        .map_err(|_| AppError::NotFound("Ordem de serviço não encontrada.".into()))?;
    carregar_itens(conn, &mut os)?;
    Ok(os)
}

pub fn listar(conn: &Connection) -> AppResult<Vec<Ordem>> {
    let mut stmt = conn.prepare(&format!("{SELECT_BASE} ORDER BY os.id DESC"))?;
    let mut ordens: Vec<Ordem> = stmt.query_map([], de_linha)?.collect::<Result<_, _>>()?;
    carregar_itens_lote(conn, &mut ordens)?;
    Ok(ordens)
}

/// Um bloco da lista de ordens (mais novas primeiro), com os itens de todas as ordens do bloco em uma consulta.
pub fn listar_pagina(conn: &Connection, pagina: crate::paginacao::Pagina) -> AppResult<Vec<Ordem>> {
    let mut stmt = conn.prepare(&format!("{SELECT_BASE} ORDER BY os.id DESC LIMIT ?1 OFFSET ?2"))?;
    let mut ordens: Vec<Ordem> = stmt.query_map(params![pagina.limite, pagina.deslocamento], de_linha)?.collect::<Result<_, _>>()?;
    carregar_itens_lote(conn, &mut ordens)?;
    Ok(ordens)
}

/// Carrega os itens de várias ordens de uma vez (antes era uma consulta por ordem).
fn carregar_itens_lote(conn: &Connection, ordens: &mut [Ordem]) -> AppResult<()> {
    if ordens.is_empty() {
        return Ok(());
    }
    let marcas = vec!["?"; ordens.len()].join(",");
    let sql = format!(
        "SELECT oi.os_id, oi.item_id, i.nome, i.tipo, oi.quantidade, oi.preco_unitario, oi.subtotal
         FROM ordem_itens oi JOIN itens i ON i.id = oi.item_id
         WHERE oi.os_id IN ({marcas}) ORDER BY oi.id"
    );
    let ids: Vec<i64> = ordens.iter().map(|o| o.id).collect();
    let mut stmt = conn.prepare(&sql)?;
    let linhas = stmt.query_map(rusqlite::params_from_iter(ids.iter()), |r| {
        Ok((
            r.get::<_, i64>(0)?,
            LinhaOrdem {
                item_id: r.get(1)?,
                item_nome: r.get(2)?,
                tipo: r.get(3)?,
                quantidade: r.get(4)?,
                preco_unitario: r.get(5)?,
                subtotal: r.get(6)?,
            },
        ))
    })?;
    let mut por_ordem: std::collections::HashMap<i64, Vec<LinhaOrdem>> = std::collections::HashMap::new();
    for linha in linhas {
        let (os_id, item) = linha?;
        por_ordem.entry(os_id).or_default().push(item);
    }
    for os in ordens.iter_mut() {
        os.itens = por_ordem.remove(&os.id).unwrap_or_default();
    }
    Ok(())
}

pub fn criar(conn: &Connection, input: &OrdemInput) -> AppResult<Ordem> {
    let mut v = Validator::new();
    let veiculo_id = v.int("veiculo_id", input.veiculo_id, true, 1, i64::MAX);
    let funcionario_id = v.int("funcionario_id", input.funcionario_id, true, 1, i64::MAX);
    let descricao = v.text("descricao", input.descricao.as_deref(), false, 1000);
    v.finish()?;
    let (veiculo_id, funcionario_id) = (veiculo_id.unwrap(), funcionario_id.unwrap());

    veiculos::buscar(conn, veiculo_id)?;
    funcionarios::buscar(conn, funcionario_id)?;
    let linhas = validar_linhas(conn, &input.itens)?;

    conn.execute_batch("BEGIN")?;
    let resultado = (|| -> AppResult<i64> {
        conn.execute(
            "INSERT INTO ordens_servico (veiculo_id, funcionario_id, descricao, status, data_entrada, valor_total)
             VALUES (?1, ?2, ?3, ?4, ?5, 0)",
            params![veiculo_id, funcionario_id, descricao, Status::Criada.como_texto(), Utc::now().to_rfc3339()],
        )?;
        let id = conn.last_insert_rowid();
        gravar_linhas(conn, id, &linhas)?;
        Ok(id)
    })();
    finalizar(conn, resultado).and_then(|id| buscar(conn, id))
}

/// Venda de balcão: cria e conclui numa só transação (baixa de estoque, forma de pagamento e pontos de fidelidade).
/// Exige caixa aberto antes de gravar qualquer coisa.
pub fn criar_balcao(conn: &Connection, input: &BalcaoInput) -> AppResult<Ordem> {
    let mut v = Validator::new();
    let funcionario_id = v.int("funcionario_id", input.funcionario_id, true, 1, i64::MAX);
    let descricao = v.text("descricao", input.descricao.as_deref(), false, 1000);
    let pagamento = input.pagamento.clone().unwrap_or_default();
    if !crate::caixa::PAGAMENTOS.contains(&pagamento.as_str()) {
        v.fail("pagamento", "Escolha dinheiro, pix, cartão ou outro.");
    }
    if input.itens.is_empty() {
        v.fail("itens", "Informe ao menos uma peça ou serviço.");
    }
    v.finish()?;
    let funcionario_id = funcionario_id.unwrap();

    funcionarios::buscar(conn, funcionario_id)?;
    if let Some(cliente) = input.cliente_id {
        clientes::buscar(conn, cliente)?;
    }
    let linhas = validar_linhas(conn, &input.itens)?;
    crate::caixa::exigir_aberto(conn)?;

    let agora = Utc::now().to_rfc3339();
    conn.execute_batch("BEGIN")?;
    let resultado = (|| -> AppResult<i64> {
        conn.execute(
            "INSERT INTO ordens_servico (tipo, veiculo_id, cliente_id, funcionario_id, descricao, status, data_entrada, valor_total)
             VALUES ('BALCAO', NULL, ?1, ?2, ?3, ?4, ?5, 0)",
            params![input.cliente_id, funcionario_id, descricao, Status::Criada.como_texto(), agora],
        )?;
        let id = conn.last_insert_rowid();
        gravar_linhas(conn, id, &linhas)?;
        let venda = buscar(conn, id)?;
        baixar_estoque(conn, id, &venda.itens)?;
        conn.execute(
            "UPDATE ordens_servico SET status = ?1, data_conclusao = ?2, pagamento = ?3 WHERE id = ?4",
            params![Status::Concluida.como_texto(), agora, pagamento, id],
        )?;
        financeiro::registrar_venda_balcao(
            conn,
            id,
            input.cliente_id,
            financeiro::centavos_de_reais(venda.valor_total),
            &pagamento,
            &venda.funcionario_nome,
        )?;
        fidelidade::registrar_compra(conn, input.cliente_id, venda.valor_total, "VENDA", id)?;
        Ok(id)
    })();
    finalizar(conn, resultado).and_then(|id| buscar(conn, id))
}

pub fn substituir_itens(conn: &Connection, id: i64, itens: &[LinhaInput]) -> AppResult<Ordem> {
    let os = buscar(conn, id)?;
    if !os.status.permite_editar_itens() {
        return Err(AppError::Conflict("Itens só podem ser alterados enquanto a ordem está CRIADA.".into()));
    }
    let linhas = validar_linhas(conn, itens)?;
    conn.execute_batch("BEGIN")?;
    let resultado = gravar_linhas(conn, id, &linhas)
        .and_then(|_| financeiro::conferir_nao_passa_total(conn, id))
        .map(|_| id);
    finalizar(conn, resultado).and_then(|id| buscar(conn, id))
}

/// Altera o status. Concluir o serviço leva a AGUARDANDO_PAGAMENTO (`concluir_servico`).
/// Encerrar (CONCLUIDA) só é possível quando a ordem está totalmente paga.
pub fn alterar_status(conn: &Connection, id: i64, novo: Status) -> AppResult<Ordem> {
    match novo {
        Status::AguardandoPagamento => concluir_servico(conn, id, None),
        Status::Concluida => encerrar(conn, id),
        _ => {
            let os = buscar(conn, id)?;
            rules::validar_transicao(os.status, novo)?;
            if novo == Status::Cancelada && financeiro::recebido_ordem(conn, id)? > 0 {
                return Err(AppError::Conflict("Esta ordem já tem pagamento registrado. Não é possível cancelar.".into()));
            }
            conn.execute("UPDATE ordens_servico SET status = ?1 WHERE id = ?2", params![novo.como_texto(), id])?;
            buscar(conn, id)
        }
    }
}

/// Conclui o serviço: EM_ANDAMENTO -> AGUARDANDO_PAGAMENTO.
/// A baixa de estoque acontece AQUI, na conclusão do serviço (não no pagamento).
/// Também abre a conta a receber (vencimento padrão: hoje).
pub fn concluir_servico(conn: &Connection, id: i64, vencimento: Option<&str>) -> AppResult<Ordem> {
    let os = buscar(conn, id)?;
    em_transacao(conn, || concluir_servico_tx(conn, &os, vencimento))?;
    buscar(conn, id)
}

fn concluir_servico_tx(conn: &Connection, os: &Ordem, vencimento: Option<&str>) -> AppResult<()> {
    rules::validar_transicao(os.status, Status::AguardandoPagamento)?;
    baixar_estoque(conn, os.id, &os.itens)?;
    conn.execute(
        "UPDATE ordens_servico SET status = ?1, data_conclusao = ?2 WHERE id = ?3",
        params![Status::AguardandoPagamento.como_texto(), Utc::now().to_rfc3339(), os.id],
    )?;
    financeiro::abrir_conta_os(conn, os.id, os.cliente_id, vencimento)
}

/// Encerra a ordem (CONCLUIDA). Exige saldo zero. Os pontos de fidelidade são dados só aqui, com a compra paga.
fn encerrar(conn: &Connection, id: i64) -> AppResult<Ordem> {
    let os = buscar(conn, id)?;
    if os.status == Status::EmAndamento {
        return Err(AppError::Conflict("Conclua o serviço antes. Depois registre o pagamento e encerre a ordem.".into()));
    }
    rules::validar_transicao(os.status, Status::Concluida)?;
    let saldo = financeiro::saldo_ordem(conn, id)?;
    if saldo > 0 {
        return Err(AppError::Conflict(format!(
            "Ainda falta receber R$ {},{:02}. Registre o pagamento antes de encerrar a ordem.",
            saldo / 100,
            saldo % 100
        )));
    }
    em_transacao(conn, || encerrar_tx(conn, &os))?;
    buscar(conn, id)
}

fn encerrar_tx(conn: &Connection, os: &Ordem) -> AppResult<()> {
    conn.execute(
        "UPDATE ordens_servico SET status = ?1 WHERE id = ?2",
        params![Status::Concluida.como_texto(), os.id],
    )?;
    let origem = if os.tipo == "BALCAO" { "VENDA" } else { "OS" };
    fidelidade::registrar_compra(conn, os.cliente_id, os.valor_total, origem, os.id)?;
    Ok(())
}

/// Conclui e recebe o restante de uma vez (atalho usado pela tela atual).
/// Passos: concluir o serviço (se ainda não concluído), receber o saldo na forma escolhida e encerrar.
pub fn concluir_com_pagamento(conn: &Connection, id: i64, pagamento: &str) -> AppResult<Ordem> {
    if !crate::caixa::PAGAMENTOS.contains(&pagamento) {
        return Err(AppError::Validation(vec![crate::error::FieldError {
            field: "pagamento".into(),
            message: "Escolha dinheiro, pix, cartão ou outro.".into(),
        }]));
    }
    let os = buscar(conn, id)?;
    if os.status == Status::Concluida {
        return Err(AppError::Conflict("Esta ordem já está concluída.".into()));
    }
    em_transacao(conn, || {
        if os.status != Status::AguardandoPagamento {
            concluir_servico_tx(conn, &os, None)?;
        }
        let saldo = financeiro::saldo_ordem(conn, id)?;
        if saldo > 0 {
            financeiro::registrar_recebimento_tx(conn, id, saldo, pagamento, &os.funcionario_nome, &financeiro::agora())?;
        }
        let atual = buscar(conn, id)?;
        encerrar_tx(conn, &atual)
    })?;
    buscar(conn, id)
}

/// Roda `f` dentro de uma transação: COMMIT se deu certo, ROLLBACK se não.
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

/// Dá baixa nas peças. Confere o estoque de todas antes; qualquer falha desfaz tudo (rollback).
/// Grava na linha da ordem o custo efetivo da saída: o custo médio vigente naquele instante (`preco_custo`,
/// que as compras atualizam). O lucro usa esse valor, então vendas antigas não mudam com compras novas.
fn baixar_estoque(conn: &Connection, os_id: i64, linhas: &[LinhaOrdem]) -> AppResult<()> {
    for linha in linhas.iter().filter(|l| l.tipo == "peca") {
        let item = crate::itens::buscar(conn, linha.item_id)?;
        if item.quantidade < linha.quantidade {
            return Err(AppError::Conflict(format!(
                "Estoque insuficiente de \"{}\": disponível {}, necessário {}.",
                linha.item_nome, item.quantidade, linha.quantidade
            )));
        }
        conn.execute(
            "UPDATE itens SET quantidade = quantidade - ?1 WHERE id = ?2",
            params![linha.quantidade, linha.item_id],
        )?;
        let custo_centavos = (item.preco_custo * 100.0).round() as i64;
        conn.execute(
            "UPDATE ordem_itens SET custo_unitario_centavos = ?1 WHERE os_id = ?2 AND item_id = ?3",
            params![custo_centavos, os_id, linha.item_id],
        )?;
    }
    Ok(())
}

pub fn excluir(conn: &Connection, id: i64) -> AppResult<()> {
    let os = buscar(conn, id)?;
    if !os.status.permite_excluir() {
        return Err(AppError::Conflict("Só é possível excluir ordens CRIADAS ou CANCELADAS.".into()));
    }
    conn.execute("DELETE FROM ordens_servico WHERE id = ?1", [id])?;
    Ok(())
}

/// Fecha a transação: COMMIT se deu certo, ROLLBACK se não.
fn finalizar(conn: &Connection, resultado: AppResult<i64>) -> AppResult<i64> {
    match resultado {
        Ok(id) => {
            conn.execute_batch("COMMIT")?;
            Ok(id)
        }
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}
