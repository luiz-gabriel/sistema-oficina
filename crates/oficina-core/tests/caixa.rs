//! Caixa: abrir com troco, receber nas ordens, fechar com conferência do dinheiro.
use oficina_core::caixa;
use oficina_core::comandos::{executar, Contexto};
use oficina_core::db;
use oficina_core::error::AppError;
use oficina_core::ordens::{self, LinhaInput, OrdemInput, Status};
use rusqlite::Connection;
use serde_json::json;

fn cenario() -> Connection {
    let conn = db::open_in_memory().unwrap();
    conn.execute_batch(
        "INSERT INTO clientes (id, nome) VALUES (1, 'Ana');
         INSERT INTO veiculos (id, cliente_id, placa, modelo) VALUES (1, 1, 'ABC1D23', 'Polo');
         INSERT INTO funcionarios (id, nome) VALUES (1, 'Carlos');
         INSERT INTO itens (id, nome, tipo, preco_venda) VALUES (2, 'Troca', 'servico', 50);",
    )
    .unwrap();
    conn
}

fn os_com_valor(conn: &Connection) -> i64 {
    let input = OrdemInput {
        veiculo_id: Some(1),
        funcionario_id: Some(1),
        descricao: None,
        itens: vec![LinhaInput { item_id: Some(2), quantidade: Some(1) }],
    };
    let id = ordens::criar(conn, &input).unwrap().id;
    ordens::alterar_status(conn, id, Status::EmAndamento).unwrap();
    id
}

fn ctx() -> Contexto {
    Contexto { usuario: "Carlos".into(), perfil: "administrador".into(), computador: "PC1".into() }
}

#[test]
fn nao_recebe_sem_caixa_aberto() {
    let conn = cenario();
    let id = os_com_valor(&conn);
    let r = executar(&conn, "ordens_alterar_status", &json!({"id": id, "status": "CONCLUIDA", "pagamento": "pix"}), &ctx());
    assert!(matches!(r, Err(AppError::Conflict(ref m)) if m.contains("Abra o caixa")), "{r:?}");
}

#[test]
fn abre_recebe_e_fecha_com_diferenca() {
    let conn = cenario();
    let abertura = executar(&conn, "caixa_abrir", &json!({"valor_inicial": 100.0}), &ctx()).unwrap();
    assert_eq!(abertura["valor_abertura"], 100.0);
    assert_eq!(abertura["aberto_por"], "Carlos");

    // Não abre dois caixas ao mesmo tempo.
    assert!(matches!(executar(&conn, "caixa_abrir", &json!({}), &ctx()), Err(AppError::Conflict(_))));

    let os = os_com_valor(&conn);
    executar(&conn, "ordens_alterar_status", &json!({"id": os, "status": "CONCLUIDA", "pagamento": "dinheiro"}), &ctx()).unwrap();
    let os2 = os_com_valor(&conn);
    executar(&conn, "ordens_alterar_status", &json!({"id": os2, "status": "CONCLUIDA", "pagamento": "pix"}), &ctx()).unwrap();

    let situacao = caixa::situacao(&conn).unwrap();
    assert!(situacao.aberto);
    assert_eq!(situacao.recebimentos.dinheiro, 50.0);
    assert_eq!(situacao.recebimentos.pix, 50.0);
    assert_eq!(situacao.recebimentos.total, 100.0);
    assert_eq!(situacao.dinheiro_esperado, 150.0); // 100 de troco + 50 em dinheiro

    // Conferiu 140: falta 10.
    let id = situacao.sessao.unwrap().id;
    let fechado = executar(&conn, "caixa_fechar", &json!({"id": id, "valor_contado": 140.0}), &ctx()).unwrap();
    assert_eq!(fechado["dinheiro_esperado"], 150.0);
    assert_eq!(fechado["diferenca"], -10.0);
    assert!(fechado["fechado_em"].is_string());

    let depois = caixa::situacao(&conn).unwrap();
    assert!(!depois.aberto);
    assert_eq!(caixa::historico(&conn, 10).unwrap().len(), 1);
}

#[test]
fn pagamento_invalido_e_recusado() {
    let conn = cenario();
    executar(&conn, "caixa_abrir", &json!({}), &ctx()).unwrap();
    let os = os_com_valor(&conn);
    let r = executar(&conn, "ordens_alterar_status", &json!({"id": os, "status": "CONCLUIDA", "pagamento": "cheque"}), &ctx());
    assert!(matches!(r, Err(AppError::Validation(_))), "{r:?}");
}

#[test]
fn fechar_sem_caixa_aberto_avisa() {
    let conn = cenario();
    let r = executar(&conn, "caixa_fechar", &json!({"id": 1, "valor_contado": 0.0}), &ctx());
    assert!(matches!(r, Err(AppError::Conflict(_))));
}
