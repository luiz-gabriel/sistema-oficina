//! Custo das peças gravado na saída: a venda guarda o custo do momento da baixa, então uma compra
//! posterior com custo maior não muda o lucro de vendas já feitas.
use chrono::Local;
use oficina_core::comandos::{executar, Contexto};
use oficina_core::compras::{self, CompraInput, LinhaInput as LinhaCompra};
use oficina_core::db;
use oficina_core::ordens::{self, LinhaInput, OrdemInput, Status};
use oficina_core::relatorios::{self, Periodo};
use rusqlite::Connection;
use serde_json::json;

fn ctx() -> Contexto {
    Contexto { usuario: "Carlos".into(), perfil: "administrador".into(), computador: "PC1".into() }
}

/// Filtro de óleo com custo cadastrado de R$ 10,00 e estoque de 5 unidades. Caixa aberto para as vendas.
fn cenario() -> Connection {
    let conn = db::open_in_memory().unwrap();
    conn.execute_batch(
        "INSERT INTO clientes (id, nome) VALUES (1, 'Ana');
         INSERT INTO veiculos (id, cliente_id, placa, modelo) VALUES (1, 1, 'ABC1D23', 'Polo');
         INSERT INTO funcionarios (id, nome) VALUES (1, 'Carlos');
         INSERT INTO fornecedores (id, nome, documento) VALUES (1, 'Peças SA', '00000000000191');
         INSERT INTO itens (id, nome, tipo, preco_custo, preco_venda, quantidade, estoque_minimo) VALUES
           (1, 'Filtro de óleo', 'peca', 10, 30, 5, 0);",
    )
    .unwrap();
    executar(&conn, "caixa_abrir", &json!({ "valor_inicial": 0.0 }), &ctx()).unwrap();
    conn
}

/// Periodo de hoje (as saídas gravadas agora caem no dia local atual).
fn hoje() -> Periodo {
    let dia = Local::now().format("%Y-%m-%d").to_string();
    Periodo::novo(&dia, &dia).unwrap()
}

fn custo_do_periodo(conn: &Connection) -> i64 {
    relatorios::lucro(conn, &hoje()).unwrap().custo_pecas_centavos
}

fn venda_balcao_de_um_filtro(conn: &Connection) {
    executar(
        conn,
        "venda_balcao_criar",
        &json!({ "dados": { "funcionario_id": 1, "pagamento": "dinheiro", "itens": [{ "item_id": 1, "quantidade": 1 }] } }),
        &ctx(),
    )
    .unwrap();
}

#[test]
fn venda_guarda_o_custo_do_momento_e_compra_posterior_nao_muda_o_lucro() {
    let conn = cenario();
    venda_balcao_de_um_filtro(&conn); // custo do cadastro: R$ 10,00
    assert_eq!(custo_do_periodo(&conn), 1_000);

    // Compra com custo maior: 4 em estoque a R$ 10,00 + 2 a R$ 16,00 = custo médio R$ 12,00.
    compras::registrar(
        &conn,
        &CompraInput {
            fornecedor_id: Some(1),
            data: Some("2026-11-20".into()),
            pagamento: Some("pix".into()),
            vencimento: None,
            itens: vec![LinhaCompra { item_id: Some(1), quantidade: Some(2), custo_unitario_centavos: Some(1_600) }],
        },
        "Carlos",
    )
    .unwrap();
    let custo_atual: f64 = conn.query_row("SELECT preco_custo FROM itens WHERE id = 1", [], |r| r.get(0)).unwrap();
    assert_eq!(custo_atual, 12.0);

    // A venda anterior continua com o custo que tinha na saída.
    assert_eq!(custo_do_periodo(&conn), 1_000, "o lucro da venda anterior não pode mudar com a compra");

    // Nova venda sai pelo custo atual (R$ 12,00): 1000 + 1200.
    venda_balcao_de_um_filtro(&conn);
    assert_eq!(custo_do_periodo(&conn), 2_200);
}

#[test]
fn conclusao_de_os_grava_o_custo_na_linha_da_peca() {
    let conn = cenario();
    let os = ordens::criar(
        &conn,
        &OrdemInput {
            veiculo_id: Some(1),
            funcionario_id: Some(1),
            descricao: None,
            itens: vec![LinhaInput { item_id: Some(1), quantidade: Some(2) }],
        },
    )
    .unwrap();
    ordens::alterar_status(&conn, os.id, Status::EmAndamento).unwrap();
    // Concluir e receber: o lucro só conta ordens CONCLUIDAS (paga e encerrada).
    ordens::concluir_com_pagamento(&conn, os.id, "dinheiro").unwrap();

    let gravado: Option<i64> = conn
        .query_row("SELECT custo_unitario_centavos FROM ordem_itens WHERE os_id = ?1 AND item_id = 1", [os.id], |r| r.get(0))
        .unwrap();
    assert_eq!(gravado, Some(1_000));
    assert_eq!(custo_do_periodo(&conn), 2_000);
}
