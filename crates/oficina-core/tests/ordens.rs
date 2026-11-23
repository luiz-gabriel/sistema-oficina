//! Regras que realmente importam: total, baixa de estoque, transições e rollback.
use oficina_core::db;
use oficina_core::error::AppError;
use oficina_core::ordens::{self, LinhaInput, OrdemInput, Status};
use rusqlite::Connection;

fn cenario() -> Connection {
    let conn = db::open_in_memory().unwrap();
    conn.execute_batch(
        "INSERT INTO clientes (id, nome) VALUES (1, 'Ana');
         INSERT INTO veiculos (id, cliente_id, placa, modelo) VALUES (1, 1, 'ABC1D23', 'Polo');
         INSERT INTO funcionarios (id, nome) VALUES (1, 'Carlos');
         INSERT INTO itens (id, nome, tipo, preco_venda, quantidade) VALUES (1, 'Filtro', 'peca', 30, 5);
         INSERT INTO itens (id, nome, tipo, preco_venda) VALUES (2, 'Troca', 'servico', 50);",
    )
    .unwrap();
    conn
}

fn linha(item_id: i64, quantidade: i64) -> LinhaInput {
    LinhaInput { item_id: Some(item_id), quantidade: Some(quantidade) }
}

fn nova_os(conn: &Connection, itens: Vec<LinhaInput>) -> i64 {
    let input = OrdemInput { veiculo_id: Some(1), funcionario_id: Some(1), descricao: None, itens };
    ordens::criar(conn, &input).unwrap().id
}

fn estoque(conn: &Connection, item: i64) -> i64 {
    oficina_core::itens::buscar(conn, item).unwrap().quantidade
}

#[test]
fn total_soma_quantidade_vezes_preco() {
    let conn = cenario();
    let os = ordens::criar(
        &conn,
        &OrdemInput {
            veiculo_id: Some(1),
            funcionario_id: Some(1),
            descricao: None,
            itens: vec![linha(1, 2), linha(2, 1)],
        },
    )
    .unwrap();
    assert_eq!(os.valor_total, 110.0); // 2×30 + 1×50
    assert_eq!(os.status, Status::Criada);
}

#[test]
fn concluir_baixa_so_o_estoque_das_pecas() {
    let conn = cenario();
    let id = nova_os(&conn, vec![linha(1, 2), linha(2, 1)]);
    ordens::alterar_status(&conn, id, Status::EmAndamento).unwrap();
    // Concluir o serviço dá a baixa no estoque e leva a AGUARDANDO_PAGAMENTO (o pagamento vem depois).
    let aguardando = ordens::alterar_status(&conn, id, Status::AguardandoPagamento).unwrap();

    assert_eq!(aguardando.status, Status::AguardandoPagamento);
    assert!(aguardando.data_conclusao.is_some());
    assert_eq!(estoque(&conn, 1), 3); // 5 − 2
    assert_eq!(estoque(&conn, 2), 0); // serviço não mexe no estoque
}

#[test]
fn estoque_insuficiente_bloqueia_conclusao_sem_alterar_nada() {
    let conn = cenario();
    let id = nova_os(&conn, vec![linha(1, 9)]);
    ordens::alterar_status(&conn, id, Status::EmAndamento).unwrap();

    let erro = ordens::alterar_status(&conn, id, Status::AguardandoPagamento).unwrap_err();
    assert!(matches!(erro, AppError::Conflict(ref m) if m.contains("Estoque insuficiente")));
    assert_eq!(estoque(&conn, 1), 5);
    assert_eq!(ordens::buscar(&conn, id).unwrap().status, Status::EmAndamento);
}

#[test]
fn ordem_concluida_e_status_final() {
    let conn = cenario();
    let id = nova_os(&conn, vec![]);
    ordens::alterar_status(&conn, id, Status::EmAndamento).unwrap();
    // Sem itens não há nada a receber: concluir e encerrar com o atalho de pagamento.
    ordens::concluir_com_pagamento(&conn, id, "pix").unwrap();

    assert!(ordens::alterar_status(&conn, id, Status::Cancelada).is_err());
    assert!(ordens::substituir_itens(&conn, id, &[]).is_err());
}
