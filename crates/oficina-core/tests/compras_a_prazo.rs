//! Compra a prazo: a conta fica só em `contas_pagar` (sem duplicar na lista, no painel e no resumo),
//! é paga em partes, e a migração de bancos antigos (`contas_pagar_compra`) gera uma conta só.
use chrono::NaiveDate;
use oficina_core::compras::{self, CompraInput, LinhaInput};
use oficina_core::db;
use oficina_core::financeiro::{self, CadastroPagar, Filtro};
use oficina_core::painel;
use oficina_core::relatorios;
use rusqlite::Connection;

fn cenario() -> Connection {
    let conn = db::open_in_memory().unwrap();
    conn.execute_batch(
        "INSERT INTO fornecedores (id, nome, documento) VALUES (1, 'Distribuidora Sul', '00000000000191');
         INSERT INTO itens (id, nome, tipo, preco_custo, preco_venda, quantidade, estoque_minimo) VALUES
           (1, 'Filtro de óleo', 'peca', 0, 30, 0, 0);",
    )
    .unwrap();
    conn
}

fn contar(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).unwrap()
}

/// Compra a prazo de `quantidade` filtros a `custo` centavos cada.
fn compra_a_prazo(conn: &Connection, data: &str, vencimento: &str, custo: i64, quantidade: i64) -> compras::Compra {
    compras::registrar(
        conn,
        &CompraInput {
            fornecedor_id: Some(1),
            data: Some(data.into()),
            pagamento: Some("a pagar".into()),
            vencimento: Some(vencimento.into()),
            itens: vec![LinhaInput { item_id: Some(1), quantidade: Some(quantidade), custo_unitario_centavos: Some(custo) }],
        },
        "Carlos",
    )
    .unwrap()
}

#[test]
fn compra_a_prazo_aparece_em_contas_a_pagar() {
    let conn = cenario();
    let c = compra_a_prazo(&conn, "2026-11-10", "2099-12-10", 1000, 5);

    let contas = financeiro::contas_pagar_listar(&conn, &Filtro::default()).unwrap();
    assert_eq!(contas.len(), 1);
    let conta = &contas[0];
    assert_eq!(conta.compra_id, Some(c.id));
    assert_eq!(conta.descricao, format!("Compra #{}", c.id));
    assert_eq!(conta.fornecedor.as_deref(), Some("Distribuidora Sul"));
    assert_eq!((conta.valor_centavos, conta.saldo_centavos, conta.pago_centavos), (5_000, 5_000, 0));
    assert_eq!((conta.vencimento.as_str(), conta.data.as_str()), ("2099-12-10", "2026-11-10"));
    assert_eq!(conta.situacao, "ABERTA");
}

#[test]
fn pagamento_parcial_e_depois_total_quita_a_compra() {
    let conn = cenario();
    let c = compra_a_prazo(&conn, "2026-11-10", "2099-12-10", 1000, 5);
    let id = financeiro::contas_pagar_listar(&conn, &Filtro::default()).unwrap()[0].id;

    let parcial = financeiro::contas_pagar_pagar(&conn, id, 2_000, Some("2026-11-12"), "Carlos").unwrap();
    assert_eq!((parcial.pago_centavos, parcial.saldo_centavos, parcial.situacao.as_str()), (2_000, 3_000, "PARCIAL"));
    assert!(!compras::buscar(&conn, c.id).unwrap().paga, "compra ainda não quitada");

    let total = financeiro::contas_pagar_pagar(&conn, id, 3_000, Some("2026-11-20"), "Carlos").unwrap();
    assert_eq!((total.saldo_centavos, total.situacao.as_str()), (0, "PAGA"));
    assert!(compras::buscar(&conn, c.id).unwrap().paga, "compra quitada quando a soma dos pagamentos chega ao valor");

    // Pagar além do saldo é recusado e não grava nada.
    assert!(financeiro::contas_pagar_pagar(&conn, id, 1, None, "Carlos").is_err());
    assert_eq!(contar(&conn, "SELECT COUNT(*) FROM pagamentos_pagar"), 2);
}

#[test]
fn painel_relatorio_e_resumo_somam_a_divida_uma_vez() {
    let conn = cenario();
    // Compra a prazo de 100,00 + despesa avulsa de 50,00 = 150,00 a pagar. Nenhuma conta pode contar em dobro.
    compra_a_prazo(&conn, "2026-11-10", "2099-12-10", 1000, 10);
    financeiro::contas_pagar_criar(
        &conn,
        &CadastroPagar {
            descricao: "Aluguel".into(),
            fornecedor: None,
            compra_id: None,
            valor_centavos: 5_000,
            vencimento: "2099-12-05".into(),
        },
        "Carlos",
    )
    .unwrap();

    let resumo = financeiro::financeiro_resumo(
        &conn,
        &Filtro { desde: Some("2026-11-01".into()), ate: Some("2026-11-30".into()), ..Default::default() },
    )
    .unwrap();
    assert_eq!((resumo.a_pagar_centavos, resumo.a_pagar_pendente_centavos), (15_000, 15_000));

    let painel = painel::serie(&conn, "2026-11-01", "2026-11-30").unwrap();
    assert_eq!((painel.contas.pagar_aberto, painel.contas.pagar_vencido), (15_000, 0));

    let hoje = NaiveDate::from_ymd_opt(2026, 11, 22).unwrap();
    let relatorio = relatorios::contas(&conn, hoje).unwrap();
    assert_eq!((relatorio.contas_a_pagar.quantidade, relatorio.contas_a_pagar.centavos), (2, 15_000));
}

#[test]
fn migracao_converte_conta_antiga_em_uma_so() {
    // Banco no formato antigo: a compra a prazo ficava só em contas_pagar_compra.
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(db::SCHEMA).unwrap();
    conn.execute_batch(
        "INSERT INTO fornecedores (id, nome, documento) VALUES (1, 'Distribuidora Sul', '00000000000191');
         INSERT INTO compras (id, fornecedor_id, data, total_centavos, pagamento, paga, criado_em) VALUES
           (1, 1, '2026-10-01', 20000, 'a pagar', 0, '2026-10-01T10:00:00+00:00'),
           (2, 1, '2026-10-05', 5000, 'a pagar', 0, '2026-10-05T10:00:00+00:00');
         INSERT INTO contas_pagar_compra (compra_id, valor_centavos, vencimento, pago) VALUES
           (1, 20000, '2099-11-01', 0),
           (2, 5000, '2026-10-20', 1);",
    )
    .unwrap();

    // Abrir o banco aplica a migração. Rodar de novo não pode duplicar nada.
    let conn = db::init(conn).unwrap();
    let conn = db::init(conn).unwrap();

    assert_eq!(contar(&conn, "SELECT COUNT(*) FROM contas_pagar WHERE compra_id = 1"), 1);
    assert_eq!(contar(&conn, "SELECT COUNT(*) FROM contas_pagar WHERE compra_id = 2"), 1);
    assert_eq!(contar(&conn, "SELECT COUNT(*) FROM contas_pagar"), 2);

    // A compra 2 estava paga: um único pagamento, com o valor total e a data da compra.
    let (valor, data): (i64, String) = conn
        .query_row("SELECT valor_centavos, data FROM pagamentos_pagar", [], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap();
    assert_eq!((valor, data.as_str()), (5_000, "2026-10-05"));
    assert_eq!(contar(&conn, "SELECT COUNT(*) FROM pagamentos_pagar"), 1);

    assert!(compras::buscar(&conn, 2).unwrap().paga);
    assert!(!compras::buscar(&conn, 1).unwrap().paga);
    assert_eq!(financeiro::contas_pagar_listar(&conn, &Filtro::default()).unwrap().len(), 2);

    // A tabela antiga não é apagada.
    assert_eq!(contar(&conn, "SELECT COUNT(*) FROM contas_pagar_compra"), 2);
}
