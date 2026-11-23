//! Fechamento do caixa: cada venda ou OS aparece numa linha, com cliente, total cobrado e o que entrou neste caixa.
use oficina_core::caixa;
use oficina_core::comandos::{executar, Contexto};
use oficina_core::db;
use rusqlite::Connection;
use serde_json::json;

fn cenario() -> Connection {
    let conn = db::open_in_memory().unwrap();
    conn.execute_batch(
        "INSERT INTO clientes (id, nome) VALUES (1, 'Maria Souza');
         INSERT INTO funcionarios (id, nome) VALUES (1, 'Carlos');
         INSERT INTO itens (id, nome, tipo, preco_venda, quantidade) VALUES (1, 'Filtro', 'peca', 40, 5);",
    )
    .unwrap();
    conn
}

fn ctx() -> Contexto {
    Contexto { usuario: "Carlos".into(), perfil: "administrador".into(), computador: "PC1".into() }
}

#[test]
fn venda_com_cliente_aparece_numa_linha_com_cliente_total_e_forma() {
    let conn = cenario();
    executar(&conn, "caixa_abrir", &json!({"valor_inicial": 50.0}), &ctx()).unwrap();
    // Venda de 80,00 (2 x 40) para Maria, paga em dinheiro.
    executar(&conn, "venda_balcao_criar", &json!({"dados": {
        "cliente_id": 1,
        "funcionario_id": 1,
        "itens": [{"item_id": 1, "quantidade": 2}],
        "pagamento": "dinheiro",
    }}), &ctx()).unwrap();

    let caixa_id: i64 = conn.query_row("SELECT id FROM caixas ORDER BY id DESC LIMIT 1", [], |r| r.get(0)).unwrap();
    let rel = caixa::relatorio(&conn, caixa_id).unwrap();
    assert_eq!(rel.recebidos.len(), 1, "uma linha por venda");
    let linha = &rel.recebidos[0];
    assert_eq!(linha.tipo, "BALCAO");
    assert_eq!(linha.cliente, "Maria Souza");
    assert!((linha.total_venda - 80.0).abs() < 0.001, "total da venda: {}", linha.total_venda);
    assert!((linha.valor - 80.0).abs() < 0.001, "recebido neste caixa: {}", linha.valor);
    assert_eq!(linha.pagamento, "dinheiro");
    assert_eq!(rel.sessao.valor_abertura, 50.0, "troco inicial aparece no relatório");
}
