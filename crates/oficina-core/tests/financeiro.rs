//! Financeiro: conclusão separada do recebimento, pagamentos parciais e divididos, contas a receber e a pagar.
use oficina_core::comandos::{executar, Contexto};
use oficina_core::db;
use oficina_core::error::AppError;
use oficina_core::financeiro;
use oficina_core::ordens::{self, Status};
use rusqlite::Connection;
use serde_json::{json, Value};

fn ctx() -> Contexto {
    Contexto { usuario: "Carlos".into(), perfil: "administrador".into(), computador: "teste".into() }
}

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

/// OS com peça de R$ 30,00 e serviço de R$ 50,00 (total 8000 centavos), já EM_ANDAMENTO.
fn os_em_andamento(conn: &Connection) -> i64 {
    let dados = json!({"dados": {"veiculo_id": 1, "funcionario_id": 1,
        "itens": [{"item_id": 1, "quantidade": 1}, {"item_id": 2, "quantidade": 1}]}});
    let id = executar(conn, "ordens_criar", &dados, &ctx()).unwrap()["id"].as_i64().unwrap();
    executar(conn, "ordens_alterar_status", &json!({"id": id, "status": "EM_ANDAMENTO"}), &ctx()).unwrap();
    id
}

fn concluir(conn: &Connection, id: i64) -> Result<Value, AppError> {
    executar(conn, "os_concluir_servico", &json!({"id": id}), &ctx())
}

fn receber(conn: &Connection, id: i64, valor: i64, forma: &str) -> Result<Value, AppError> {
    executar(conn, "os_registrar_recebimento", &json!({"ordem_id": id, "valor_centavos": valor, "forma": forma}), &ctx())
}

fn conta_da_os(conn: &Connection, id: i64) -> Value {
    executar(conn, "contas_receber_listar", &json!({}), &ctx())
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["referencia"] == id)
        .cloned()
        .expect("conta da OS")
}

fn estoque(conn: &Connection, item: i64) -> i64 {
    oficina_core::itens::buscar(conn, item).unwrap().quantidade
}

#[test]
fn conclusao_sem_pagar_fica_aguardando_pagamento() {
    let conn = cenario();
    let id = os_em_andamento(&conn);

    let os = concluir(&conn, id).unwrap();
    assert_eq!(os["status"], "AGUARDANDO_PAGAMENTO");
    assert_eq!(estoque(&conn, 1), 4); // baixa feita na conclusão do serviço

    let conta = conta_da_os(&conn, id);
    assert_eq!(conta["valor_total_centavos"], 8000);
    assert_eq!(conta["saldo_centavos"], 8000);
    assert_eq!(conta["situacao"], "ABERTA");

    // Sem pagamento total não encerra a ordem.
    let erro = ordens::alterar_status(&conn, id, Status::Concluida).unwrap_err();
    assert!(matches!(erro, AppError::Conflict(_)));
    assert_eq!(ordens::buscar(&conn, id).unwrap().status, Status::AguardandoPagamento);
}

#[test]
fn pagamento_parcial_nao_muda_o_status_e_aparece_no_resumo() {
    let conn = cenario();
    let id = os_em_andamento(&conn);
    concluir(&conn, id).unwrap();

    let rec = receber(&conn, id, 3000, "pix").unwrap();
    assert_eq!(rec["valor_centavos"], 3000);
    assert_eq!(ordens::buscar(&conn, id).unwrap().status, Status::AguardandoPagamento);

    let conta = conta_da_os(&conn, id);
    assert_eq!(conta["saldo_centavos"], 5000);
    assert_eq!(conta["recebido_centavos"], 3000);
    assert_eq!(conta["situacao"], "PARCIAL");

    // Faturado, recebido e pendente são valores diferentes.
    let resumo = executar(&conn, "financeiro_resumo", &json!({}), &ctx()).unwrap();
    assert_eq!(resumo["faturado_centavos"], 8000);
    assert_eq!(resumo["recebido_centavos"], 3000);
    assert_eq!(resumo["pendente_centavos"], 5000);

    let so_parciais = executar(&conn, "contas_receber_listar", &json!({"situacao": "PARCIAL"}), &ctx()).unwrap();
    assert_eq!(so_parciais.as_array().unwrap().len(), 1);
}

#[test]
fn pagamento_dividido_em_formas_quita_e_encerra() {
    let conn = cenario();
    let id = os_em_andamento(&conn);
    concluir(&conn, id).unwrap();

    receber(&conn, id, 5000, "dinheiro").unwrap();
    receber(&conn, id, 3000, "cartao").unwrap();

    assert_eq!(conta_da_os(&conn, id)["situacao"], "QUITADA");
    ordens::alterar_status(&conn, id, Status::Concluida).unwrap();
    assert_eq!(ordens::buscar(&conn, id).unwrap().status, Status::Concluida);

    let lista = executar(&conn, "recebimentos_listar", &json!({"ordem_id": id}), &ctx()).unwrap();
    assert_eq!(lista.as_array().unwrap().len(), 2);
}

#[test]
fn recebimento_duplicado_na_mesma_hora_e_recusado() {
    let conn = cenario();
    let id = os_em_andamento(&conn);
    concluir(&conn, id).unwrap();

    receber(&conn, id, 2000, "pix").unwrap();
    let erro = receber(&conn, id, 2000, "pix").unwrap_err();
    assert!(matches!(erro, AppError::Conflict(_)));
    assert_eq!(conta_da_os(&conn, id)["saldo_centavos"], 6000);
}

#[test]
fn recebimento_nao_passa_do_saldo_nem_grava_valor_invalido() {
    let conn = cenario();
    let id = os_em_andamento(&conn);
    concluir(&conn, id).unwrap();

    assert!(matches!(receber(&conn, id, 9000, "pix").unwrap_err(), AppError::Validation(_)));
    assert!(matches!(receber(&conn, id, 0, "pix").unwrap_err(), AppError::Validation(_)));
    assert!(matches!(receber(&conn, id, -500, "pix").unwrap_err(), AppError::Validation(_)));
    assert!(matches!(receber(&conn, id, 100, "cheque").unwrap_err(), AppError::Validation(_)));

    let total: i64 = conn.query_row("SELECT COUNT(*) FROM recebimentos", [], |r| r.get(0)).unwrap();
    assert_eq!(total, 0);
    assert_eq!(conta_da_os(&conn, id)["saldo_centavos"], 8000);
}

#[test]
fn so_existe_uma_conta_por_origem() {
    let conn = cenario();
    let id = os_em_andamento(&conn);
    concluir(&conn, id).unwrap();

    // Abrir a conta de novo para a mesma OS é recusado.
    let erro = financeiro::abrir_conta_os(&conn, id, Some(1), None).unwrap_err();
    assert!(matches!(erro, AppError::Conflict(_)));

    // E o próprio banco também recusa uma segunda linha para a mesma origem e referência.
    let repetida = conn.execute(
        "INSERT INTO contas_receber (origem, referencia, valor_total_centavos, saldo_centavos, vencimento, data)
         VALUES ('OS', ?1, 1, 1, '2030-01-01', '2030-01-01')",
        [id],
    );
    assert!(repetida.is_err());
}

#[test]
fn conta_vencida_continua_vencida_com_pagamento_parcial() {
    let conn = cenario();
    let id = os_em_andamento(&conn);
    executar(&conn, "os_concluir_servico", &json!({"id": id, "vencimento": "2020-01-10"}), &ctx()).unwrap();
    assert_eq!(conta_da_os(&conn, id)["situacao"], "VENCIDA");

    receber(&conn, id, 1000, "pix").unwrap();
    assert_eq!(conta_da_os(&conn, id)["situacao"], "VENCIDA");
}

#[test]
fn caixa_soma_cada_recebimento_uma_vez_so() {
    let conn = cenario();
    executar(&conn, "caixa_abrir", &json!({"valor_inicial": 100.0}), &ctx()).unwrap();
    let id = os_em_andamento(&conn);
    concluir(&conn, id).unwrap();

    receber(&conn, id, 5000, "dinheiro").unwrap();
    receber(&conn, id, 3000, "pix").unwrap();
    let situacao = executar(&conn, "caixa_situacao", &json!({}), &ctx()).unwrap();
    assert_eq!(situacao["recebimentos"]["total"], 80.0);
    assert_eq!(situacao["recebimentos"]["dinheiro"], 50.0);
    assert_eq!(situacao["dinheiro_esperado"], 150.0); // troco 100 + dinheiro 50; pix não entra na gaveta

    // Encerrar a OS não soma o valor de novo no caixa.
    ordens::alterar_status(&conn, id, Status::Concluida).unwrap();
    let depois = executar(&conn, "caixa_situacao", &json!({}), &ctx()).unwrap();
    assert_eq!(depois["recebimentos"]["total"], 80.0);
    assert_eq!(depois["recebimentos"]["atendimentos"], 1);
}

#[test]
fn conta_a_pagar_aceita_pagamento_parcial_e_total() {
    let conn = cenario();
    let dados = json!({"dados": {"descricao": "Conta de luz", "fornecedor": "Energia", "valor_centavos": 15000, "vencimento": "2099-01-10"}});
    let conta = executar(&conn, "contas_pagar_criar", &dados, &ctx()).unwrap();
    let id = conta["id"].as_i64().unwrap();
    assert_eq!(conta["situacao"], "ABERTA");

    let parcial = executar(&conn, "contas_pagar_pagar", &json!({"id": id, "valor_centavos": 5000}), &ctx()).unwrap();
    assert_eq!(parcial["situacao"], "PARCIAL");
    assert_eq!(parcial["saldo_centavos"], 10000);

    let passou = executar(&conn, "contas_pagar_pagar", &json!({"id": id, "valor_centavos": 11000}), &ctx());
    assert!(matches!(passou.unwrap_err(), AppError::Validation(_)));

    let total = executar(&conn, "contas_pagar_pagar", &json!({"id": id, "valor_centavos": 10000, "data": "2026-11-20"}), &ctx()).unwrap();
    assert_eq!(total["situacao"], "PAGA");
    assert_eq!(total["saldo_centavos"], 0);

    let de_novo = executar(&conn, "contas_pagar_pagar", &json!({"id": id, "valor_centavos": 1}), &ctx());
    assert!(matches!(de_novo.unwrap_err(), AppError::Conflict(_)));

    let pagas = executar(&conn, "contas_pagar_listar", &json!({"situacao": "PAGA"}), &ctx()).unwrap();
    assert_eq!(pagas.as_array().unwrap().len(), 1);
}

#[test]
fn conta_a_pagar_vencida_aparece_como_vencida() {
    let conn = cenario();
    let dados = json!({"dados": {"descricao": "Aluguel", "valor_centavos": 90000, "vencimento": "2020-05-05"}});
    let conta = executar(&conn, "contas_pagar_criar", &dados, &ctx()).unwrap();
    assert_eq!(conta["situacao"], "VENCIDA");
}
