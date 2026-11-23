//! Fidelidade: pontos gerados por OS e vendas de balcão pagas, saldo, extrato e snapshot de sync.
use oficina_core::comandos::{executar, Contexto};
use oficina_core::db;
use oficina_core::error::AppError;
use oficina_core::fidelidade;
use oficina_core::sync;
use rusqlite::Connection;
use serde_json::json;

fn cenario() -> Connection {
    let conn = db::open_in_memory().unwrap();
    conn.execute_batch(
        "INSERT INTO clientes (id, nome) VALUES (1, 'Ana');
         INSERT INTO clientes (id, nome) VALUES (2, 'Bruno');
         INSERT INTO veiculos (id, cliente_id, placa, modelo) VALUES (1, 1, 'ABC1D23', 'Polo');
         INSERT INTO funcionarios (id, nome) VALUES (1, 'Carlos');
         INSERT INTO itens (id, nome, tipo, preco_venda, quantidade) VALUES (1, 'Filtro', 'peca', 30, 50);
         INSERT INTO itens (id, nome, tipo, preco_venda) VALUES (2, 'Troca', 'servico', 50);",
    )
    .unwrap();
    executar(&conn, "caixa_abrir", &json!({"valor_inicial": 0}), &ctx()).unwrap();
    conn
}

fn ctx() -> Contexto {
    Contexto { usuario: "Carlos".into(), perfil: "administrador".into(), computador: "PC1".into() }
}

fn saldo(conn: &Connection, cliente: i64) -> i64 {
    executar(conn, "fidelidade_saldo", &json!({"cliente_id": cliente}), &ctx()).unwrap()["saldo"].as_i64().unwrap()
}

#[test]
fn venda_de_balcao_com_cliente_gera_pontos() {
    let conn = cenario();
    // Serviço de R$ 50,00 pago com cliente identificado: 5 pontos.
    executar(
        &conn,
        "venda_balcao_criar",
        &json!({"dados": {"cliente_id": 1, "funcionario_id": 1, "pagamento": "pix", "itens": [{"item_id": 2, "quantidade": 1}]}}),
        &ctx(),
    )
    .unwrap();

    assert_eq!(saldo(&conn, 1), 5);
    let extrato = executar(&conn, "fidelidade_extrato", &json!({"cliente_id": 1}), &ctx()).unwrap();
    assert_eq!(extrato.as_array().unwrap().len(), 1);
    assert_eq!(extrato[0]["pontos"], 5);
    assert_eq!(extrato[0]["origem"], "VENDA");
    assert!(extrato[0]["criado_em"].is_string());
}

#[test]
fn venda_sem_cliente_nao_gera_pontos() {
    let conn = cenario();
    executar(
        &conn,
        "venda_balcao_criar",
        &json!({"dados": {"funcionario_id": 1, "pagamento": "dinheiro", "itens": [{"item_id": 2, "quantidade": 2}]}}),
        &ctx(),
    )
    .unwrap();
    let total: i64 = conn.query_row("SELECT COUNT(*) FROM fidelidade_movimentos", [], |r| r.get(0)).unwrap();
    assert_eq!(total, 0);
}

#[test]
fn os_concluida_gera_pontos_pelo_cliente_do_veiculo() {
    let conn = cenario();
    // Peça de R$ 30,00 (3 pontos) + serviço de R$ 50,00 (5 pontos), na OS do veículo da Ana.
    let os = executar(
        &conn,
        "ordens_criar",
        &json!({"dados": {"veiculo_id": 1, "funcionario_id": 1, "itens": [{"item_id": 1, "quantidade": 1}, {"item_id": 2, "quantidade": 1}]}}),
        &ctx(),
    )
    .unwrap()["id"]
        .as_i64()
        .unwrap();
    executar(&conn, "ordens_alterar_status", &json!({"id": os, "status": "EM_ANDAMENTO"}), &ctx()).unwrap();
    executar(&conn, "ordens_alterar_status", &json!({"id": os, "status": "CONCLUIDA", "pagamento": "cartao"}), &ctx()).unwrap();

    // R$ 80,00 pagos: 8 pontos, origem OS.
    assert_eq!(saldo(&conn, 1), 8);
    let extrato = fidelidade::extrato(&conn, 1).unwrap();
    assert_eq!(extrato[0].origem, "OS");
    assert_eq!(extrato[0].referencia, Some(os));
}

#[test]
fn saldo_e_extrato_sao_por_cliente() {
    let conn = cenario();
    executar(
        &conn,
        "venda_balcao_criar",
        &json!({"dados": {"cliente_id": 2, "funcionario_id": 1, "pagamento": "pix", "itens": [{"item_id": 2, "quantidade": 2}]}}),
        &ctx(),
    )
    .unwrap(); // R$ 100,00 para o Bruno
    assert_eq!(saldo(&conn, 1), 0);
    assert_eq!(saldo(&conn, 2), 10);
    assert!(fidelidade::extrato(&conn, 1).unwrap().is_empty());
}

#[test]
fn cliente_inexistente_nao_tem_saldo() {
    let conn = cenario();
    let r = executar(&conn, "fidelidade_saldo", &json!({"cliente_id": 999}), &ctx());
    assert!(matches!(r, Err(AppError::NotFound(_))), "{r:?}");
}

#[test]
fn resgate_usa_o_saldo_real_do_cliente() {
    let conn = cenario();
    // Duas vendas de R$ 30,00 geram 6 pontos: não dá para resgatar 100. Com 120 pontos, o resgate de 100 vale R$ 10,00.
    for _ in 0..2 {
        executar(
            &conn,
            "venda_balcao_criar",
            &json!({"dados": {"cliente_id": 1, "funcionario_id": 1, "pagamento": "pix", "itens": [{"item_id": 1, "quantidade": 1}]}}),
            &ctx(),
        )
        .unwrap();
    }
    assert_eq!(saldo(&conn, 1), 6); // 30 reais = 3 pontos por venda
    assert!(matches!(fidelidade::validar_resgate(saldo(&conn, 1), 100), Err(AppError::Conflict(_))));
    assert_eq!(fidelidade::validar_resgate(120, 100).unwrap(), 10.0);
}

#[test]
fn movimentos_entram_no_snapshot_de_sincronia() {
    let conn = cenario();
    executar(
        &conn,
        "venda_balcao_criar",
        &json!({"dados": {"cliente_id": 1, "funcionario_id": 1, "pagamento": "pix", "itens": [{"item_id": 2, "quantidade": 1}]}}),
        &ctx(),
    )
    .unwrap();
    let snap = sync::snapshot(&conn).unwrap();
    assert_eq!(snap["fidelidade_movimentos"].as_array().unwrap().len(), 1);
}
