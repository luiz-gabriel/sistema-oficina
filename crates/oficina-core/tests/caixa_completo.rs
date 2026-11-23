//! Caixa completo: sangria, suprimento, fechamento com movimentos, relatório e cupom.
use oficina_core::caixa;
use oficina_core::comandos::{executar, Contexto};
use oficina_core::db;
use oficina_core::error::AppError;
use oficina_core::impressao;
use oficina_core::ordens::{self, LinhaInput, OrdemInput, Status};
use oficina_core::sync;
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

fn ctx() -> Contexto {
    Contexto { usuario: "Carlos".into(), perfil: "administrador".into(), computador: "PC1".into() }
}

/// Cria uma OS de R$ 50 e a deixa em andamento, pronta para receber.
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

fn recebe(conn: &Connection, forma: &str) -> i64 {
    let os = os_com_valor(conn);
    executar(conn, "ordens_alterar_status", &json!({"id": os, "status": "CONCLUIDA", "pagamento": forma}), &ctx()).unwrap();
    os
}

fn id_caixa_aberto(conn: &Connection) -> i64 {
    caixa::situacao(conn).unwrap().sessao.unwrap().id
}

#[test]
fn sangria_maior_que_o_dinheiro_disponivel_e_recusada() {
    let conn = cenario();
    executar(&conn, "caixa_abrir", &json!({"valor_inicial": 100.0}), &ctx()).unwrap();
    recebe(&conn, "pix"); // PIX não entra no dinheiro da gaveta.

    let r = executar(&conn, "caixa_sangria", &json!({"valor": 100.01, "motivo": "Troco para o caixa"}), &ctx());
    assert!(matches!(r, Err(AppError::Validation(_))), "{r:?}");
    assert!(caixa::situacao(&conn).unwrap().movimentos.is_empty(), "sangria recusada não pode ser gravada");

    // Exatamente o disponível é aceito e zera a gaveta.
    let ok = executar(&conn, "caixa_sangria", &json!({"valor": 100.0, "motivo": "Depósito no banco"}), &ctx()).unwrap();
    assert_eq!(ok["tipo"], "sangria");
    assert_eq!(caixa::situacao(&conn).unwrap().dinheiro_esperado, 0.0);
}

#[test]
fn sangria_e_suprimento_com_caixa_fechado_sao_recusados() {
    let conn = cenario();
    let sem_caixa = executar(&conn, "caixa_sangria", &json!({"valor": 10.0, "motivo": "x"}), &ctx());
    assert!(matches!(sem_caixa, Err(AppError::Conflict(_))), "{sem_caixa:?}");
    let sem_caixa = executar(&conn, "caixa_suprimento", &json!({"valor": 10.0, "motivo": "x"}), &ctx());
    assert!(matches!(sem_caixa, Err(AppError::Conflict(_))), "{sem_caixa:?}");

    executar(&conn, "caixa_abrir", &json!({}), &ctx()).unwrap();
    let id = id_caixa_aberto(&conn);
    executar(&conn, "caixa_fechar", &json!({"id": id, "valor_contado": 0.0}), &ctx()).unwrap();

    let depois = executar(&conn, "caixa_suprimento", &json!({"valor": 10.0, "motivo": "Troco"}), &ctx());
    assert!(matches!(depois, Err(AppError::Conflict(_))), "{depois:?}");
    let depois = executar(&conn, "caixa_sangria", &json!({"valor": 1.0, "motivo": "Troco"}), &ctx());
    assert!(matches!(depois, Err(AppError::Conflict(_))), "{depois:?}");
    let total: i64 = conn.query_row("SELECT COUNT(*) FROM caixa_movimentos", [], |r| r.get(0)).unwrap();
    assert_eq!(total, 0);
}

#[test]
fn valor_e_motivo_sao_obrigatorios() {
    let conn = cenario();
    executar(&conn, "caixa_abrir", &json!({"valor_inicial": 50.0}), &ctx()).unwrap();
    for args in [
        json!({"motivo": "Troco"}),                       // sem valor
        json!({"valor": 0.0, "motivo": "Troco"}),         // zero
        json!({"valor": -5.0, "motivo": "Troco"}),        // negativo
        json!({"valor": 5.0}),                            // sem motivo
        json!({"valor": 5.0, "motivo": "   "}),           // motivo só com espaços
    ] {
        let r = executar(&conn, "caixa_suprimento", &args, &ctx());
        assert!(matches!(r, Err(AppError::Validation(_))), "{args} -> {r:?}");
    }
    assert!(caixa::situacao(&conn).unwrap().movimentos.is_empty());
}

#[test]
fn movimento_grava_quem_fez_e_quando() {
    let conn = cenario();
    executar(&conn, "caixa_abrir", &json!({"valor_inicial": 20.0}), &ctx()).unwrap();
    let mov = executar(&conn, "caixa_suprimento", &json!({"valor": 30.5, "motivo": "  Reforço de troco  "}), &ctx()).unwrap();
    assert_eq!(mov["usuario"], "Carlos");
    assert_eq!(mov["motivo"], "Reforço de troco");
    assert!(mov["criado_em"].is_string());
    assert_eq!(mov["valor"], 30.5);
}

#[test]
fn diferenca_do_fechamento_considera_sangrias_e_suprimentos() {
    let conn = cenario();
    executar(&conn, "caixa_abrir", &json!({"valor_inicial": 100.0}), &ctx()).unwrap();
    recebe(&conn, "dinheiro"); // +50 em dinheiro
    executar(&conn, "caixa_suprimento", &json!({"valor": 20.0, "motivo": "Troco do cofre"}), &ctx()).unwrap();
    executar(&conn, "caixa_sangria", &json!({"valor": 30.0, "motivo": "Pagamento de fornecedor"}), &ctx()).unwrap();

    let situacao = caixa::situacao(&conn).unwrap();
    assert_eq!(situacao.sangrias, 30.0);
    assert_eq!(situacao.suprimentos, 20.0);
    assert_eq!(situacao.dinheiro_esperado, 140.0); // 100 + 50 + 20 - 30
    assert_eq!(situacao.movimentos.len(), 2);

    let id = id_caixa_aberto(&conn);
    let fechado = executar(&conn, "caixa_fechar", &json!({"id": id, "valor_contado": 135.0}), &ctx()).unwrap();
    assert_eq!(fechado["dinheiro_esperado"], 140.0);
    assert_eq!(fechado["diferenca"], -5.0);
}

#[test]
fn caixa_nao_pode_ser_fechado_duas_vezes() {
    let conn = cenario();
    executar(&conn, "caixa_abrir", &json!({"valor_inicial": 10.0}), &ctx()).unwrap();
    let id = id_caixa_aberto(&conn);
    executar(&conn, "caixa_fechar", &json!({"id": id, "valor_contado": 10.0}), &ctx()).unwrap();

    let de_novo = executar(&conn, "caixa_fechar", &json!({"id": id, "valor_contado": 99.0}), &ctx());
    assert!(matches!(de_novo, Err(AppError::Conflict(_))), "{de_novo:?}");
    // O primeiro fechamento continua valendo.
    let fechado = caixa::historico(&conn, 1).unwrap().remove(0);
    assert_eq!(fechado.valor_contado, Some(10.0));
    assert_eq!(fechado.diferenca, Some(0.0));
}

#[test]
fn relatorio_do_fechamento_traz_o_resumo_completo() {
    let conn = cenario();
    executar(&conn, "caixa_abrir", &json!({"valor_inicial": 100.0}), &ctx()).unwrap();
    let os_dinheiro = recebe(&conn, "dinheiro");
    let os_pix = recebe(&conn, "pix");
    executar(&conn, "caixa_sangria", &json!({"valor": 40.0, "motivo": "Depósito"}), &ctx()).unwrap();
    executar(&conn, "caixa_suprimento", &json!({"valor": 10.0, "motivo": "Troco"}), &ctx()).unwrap();

    let id = id_caixa_aberto(&conn);
    executar(&conn, "caixa_fechar", &json!({"id": id, "valor_contado": 120.0, "observacao": "Conferido"}), &ctx()).unwrap();

    let r = caixa::relatorio(&conn, id).unwrap();
    assert_eq!(r.sessao.valor_abertura, 100.0);
    assert_eq!(r.recebimentos.dinheiro, 50.0);
    assert_eq!(r.recebimentos.pix, 50.0);
    assert_eq!(r.recebimentos.total, 100.0);
    assert_eq!(r.recebimentos.atendimentos, 2);
    assert_eq!(r.sangrias, 40.0);
    assert_eq!(r.suprimentos, 10.0);
    assert_eq!(r.dinheiro_esperado, 120.0); // 100 + 50 + 10 - 40
    assert_eq!(r.dinheiro_contado, Some(120.0));
    assert_eq!(r.diferenca, Some(0.0));

    // Ordens recebidas, com número, valor e forma.
    assert_eq!(r.recebidos.len(), 2);
    let dinheiro = r.recebidos.iter().find(|o| o.os_id == os_dinheiro).unwrap();
    assert_eq!((dinheiro.valor, dinheiro.pagamento.as_str()), (50.0, "dinheiro"));
    let pix = r.recebidos.iter().find(|o| o.os_id == os_pix).unwrap();
    assert_eq!((pix.valor, pix.pagamento.as_str()), (50.0, "pix"));

    assert_eq!(r.movimentos.len(), 2);
    assert_eq!(r.movimentos[0].tipo, "sangria");
    assert_eq!(r.movimentos[1].tipo, "suprimento");
    assert_eq!(r.sessao.observacao.as_deref(), Some("Conferido"));
}

#[test]
fn relatorio_de_caixa_aberto_e_de_caixa_inexistente() {
    let conn = cenario();
    executar(&conn, "caixa_abrir", &json!({"valor_inicial": 5.0}), &ctx()).unwrap();
    recebe(&conn, "dinheiro");
    let id = id_caixa_aberto(&conn);
    let r = caixa::relatorio(&conn, id).unwrap();
    assert!(r.sessao.fechado_em.is_none());
    assert_eq!(r.dinheiro_esperado, 55.0);
    assert_eq!(r.recebidos.len(), 1);

    let nao_existe = caixa::relatorio(&conn, 999);
    assert!(matches!(nao_existe, Err(AppError::NotFound(_))), "{nao_existe:?}");
}

#[test]
fn situacao_aberta_mostra_movimentos_e_totais() {
    let conn = cenario();
    executar(&conn, "caixa_abrir", &json!({"valor_inicial": 10.0}), &ctx()).unwrap();
    executar(&conn, "caixa_suprimento", &json!({"valor": 5.0, "motivo": "Entrada"}), &ctx()).unwrap();
    executar(&conn, "caixa_sangria", &json!({"valor": 3.0, "motivo": "Saída"}), &ctx()).unwrap();
    let s = caixa::situacao(&conn).unwrap();
    assert!(s.aberto);
    assert_eq!((s.sangrias, s.suprimentos), (3.0, 5.0));
    assert_eq!(s.dinheiro_esperado, 12.0);
    assert_eq!(s.movimentos.iter().map(|m| m.tipo.as_str()).collect::<Vec<_>>(), ["suprimento", "sangria"]);
}

#[test]
fn movimentos_entram_no_snapshot_de_sincronizacao() {
    let conn = cenario();
    executar(&conn, "caixa_abrir", &json!({"valor_inicial": 10.0}), &ctx()).unwrap();
    executar(&conn, "caixa_suprimento", &json!({"valor": 5.0, "motivo": "Entrada"}), &ctx()).unwrap();
    let snap = sync::snapshot(&conn).unwrap();
    assert_eq!(snap["caixa_movimentos"].as_array().unwrap().len(), 1);
    assert_eq!(snap["caixas"].as_array().unwrap().len(), 1);
}

#[test]
fn cupom_de_fechamento_monta_bytes_para_a_impressora() {
    let conn = cenario();
    executar(&conn, "caixa_abrir", &json!({"valor_inicial": 100.0}), &ctx()).unwrap();
    recebe(&conn, "dinheiro");
    executar(&conn, "caixa_sangria", &json!({"valor": 20.0, "motivo": "Depósito"}), &ctx()).unwrap();
    let id = id_caixa_aberto(&conn);
    executar(&conn, "caixa_fechar", &json!({"id": id, "valor_contado": 130.0}), &ctx()).unwrap();

    let r = caixa::relatorio(&conn, id).unwrap();
    let bytes = impressao::cupom_fechamento(&r).unwrap();
    assert_eq!(&bytes[..2], &[0x1B, 0x40], "começa com ESC @");
    assert!(bytes.windows(2).any(|w| w == [0x1D, 0x56]), "termina com corte");
    let texto = String::from_utf8_lossy(&bytes);
    assert!(texto.contains("Fechamento de caixa No"));
    assert!(texto.contains("Sangria"));
}

#[test]
fn comando_relatorio_devolve_o_resumo_pela_fila_de_comandos() {
    let conn = cenario();
    executar(&conn, "caixa_abrir", &json!({}), &ctx()).unwrap();
    let id = id_caixa_aberto(&conn);
    let r = executar(&conn, "caixa_relatorio", &json!({"id": id}), &ctx()).unwrap();
    assert_eq!(r["sessao"]["id"], id);
    assert!(r["recebidos"].is_array());
    assert!(r["movimentos"].is_array());
}
