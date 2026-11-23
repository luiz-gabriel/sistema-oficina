//! Orçamentos: transições, valores em centavos, congelamento, conversão em OS e aditivo.
use oficina_core::comandos::{self, Contexto};
use oficina_core::db;
use oficina_core::error::AppError;
use oficina_core::ordens::{self, LinhaInput, OrdemInput};
use oficina_core::orcamentos::{self, AditivoInput, OrcamentoInput, Status};
use rusqlite::Connection;
use serde_json::json;

const VALIDADE_FUTURA: &str = "2099-12-31";

fn cenario() -> Connection {
    let conn = db::open_in_memory().unwrap();
    conn.execute_batch(
        "INSERT INTO clientes (id, nome) VALUES (1, 'Ana');
         INSERT INTO clientes (id, nome) VALUES (2, 'Bruno');
         INSERT INTO veiculos (id, cliente_id, placa, modelo) VALUES (1, 1, 'ABC1D23', 'Polo');
         INSERT INTO veiculos (id, cliente_id, placa, modelo) VALUES (2, 2, 'XYZ9K88', 'Gol');
         INSERT INTO funcionarios (id, nome) VALUES (1, 'Carlos');
         INSERT INTO itens (id, nome, tipo, preco_venda, quantidade) VALUES (1, 'Filtro', 'peca', 30, 5);
         INSERT INTO itens (id, nome, tipo, preco_venda) VALUES (2, 'Troca de oleo', 'servico', 50);",
    )
    .unwrap();
    conn
}

fn linha(item_id: i64, quantidade: i64) -> LinhaInput {
    LinhaInput { item_id: Some(item_id), quantidade: Some(quantidade) }
}

/// Orçamento para Ana/Polo: 2 filtros (60,00) + 1 troca de óleo (50,00) = 110,00.
fn novo_orcamento(conn: &Connection) -> i64 {
    let input = OrcamentoInput {
        cliente_id: Some(1),
        veiculo_id: Some(1),
        funcionario_id: Some(1),
        observacoes: Some("Revisão completa".into()),
        validade: Some(VALIDADE_FUTURA.into()),
        prazo_dias: Some(2),
        itens: vec![linha(1, 2), linha(2, 1)],
    };
    orcamentos::criar(conn, &input, "2026-11-22T10:00:00+00:00").unwrap().id
}

fn aprovar_orcamento(conn: &Connection, id: i64) {
    orcamentos::enviar(conn, id, "2026-11-22T10:05:00+00:00").unwrap();
    orcamentos::aprovar(conn, id, "telefone", None, "2026-11-22T11:00:00+00:00").unwrap();
}

#[test]
fn so_cinco_transicoes_sao_validas() {
    use Status::*;
    let todos = [Rascunho, AguardandoAprovacao, Aprovado, Recusado, Cancelado];
    let mut validas = Vec::new();
    for de in todos {
        for para in todos {
            if de.pode_transitar_para(para) {
                validas.push((de, para));
            }
        }
    }
    assert_eq!(
        validas,
        vec![
            (Rascunho, AguardandoAprovacao),
            (Rascunho, Cancelado),
            (AguardandoAprovacao, Aprovado),
            (AguardandoAprovacao, Recusado),
            (AguardandoAprovacao, Cancelado),
        ]
    );
}

#[test]
fn aprovado_recusado_e_cancelado_sao_finais() {
    for de in [Status::Aprovado, Status::Recusado, Status::Cancelado] {
        for para in [Status::Rascunho, Status::AguardandoAprovacao, Status::Aprovado, Status::Recusado, Status::Cancelado] {
            assert!(!de.pode_transitar_para(para), "{de:?} não deveria ir para {para:?}");
        }
    }
    let erro = orcamentos::validar_transicao(Status::Aprovado, Status::Cancelado).unwrap_err();
    assert!(matches!(erro, AppError::Conflict(_)));
}

#[test]
fn status_vai_e_volta_pelo_texto_do_banco() {
    for s in [Status::Rascunho, Status::AguardandoAprovacao, Status::Aprovado, Status::Recusado, Status::Cancelado] {
        assert_eq!(Status::de_texto(s.como_texto()), Some(s));
    }
    assert_eq!(Status::de_texto("FINALIZADO"), None);
}

#[test]
fn valores_em_centavos_sem_erro_de_arredondamento() {
    assert_eq!(orcamentos::centavos(19.90), 1990);
    assert_eq!(orcamentos::centavos(10.99), 1099);
    assert_eq!(orcamentos::centavos(0.1 + 0.2), 30);
    assert_eq!(orcamentos::subtotal_centavos(3, 1990), 5970);
    assert_eq!(orcamentos::total_centavos(&[5970, 1099]), 7069);
}

#[test]
fn criar_soma_total_em_centavos_e_comeca_como_rascunho() {
    let conn = cenario();
    let id = novo_orcamento(&conn);
    let orc = orcamentos::buscar(&conn, id).unwrap();
    assert_eq!(orc.status, Status::Rascunho);
    assert_eq!(orc.valor_total_centavos, 11000);
    assert_eq!(orc.itens.len(), 2);
    assert_eq!(orc.itens[0].preco_unitario_centavos, 3000);
    assert_eq!(orc.itens[0].subtotal_centavos, 6000);
    assert_eq!(orc.cliente_nome, "Ana");
    assert_eq!(orc.veiculo_placa, "ABC1D23");
    assert_eq!(orc.prazo_dias, Some(2));
}

#[test]
fn criar_recusa_veiculo_de_outro_cliente_validade_passada_e_lista_vazia() {
    let conn = cenario();
    let mut input = OrcamentoInput {
        cliente_id: Some(1),
        veiculo_id: Some(2),
        funcionario_id: Some(1),
        validade: Some(VALIDADE_FUTURA.into()),
        itens: vec![linha(1, 1)],
        ..Default::default()
    };
    assert!(matches!(orcamentos::criar(&conn, &input, "x").unwrap_err(), AppError::Validation(_)));

    input.veiculo_id = Some(1);
    input.validade = Some("2000-01-01".into());
    assert!(matches!(orcamentos::criar(&conn, &input, "x").unwrap_err(), AppError::Validation(_)));

    input.validade = Some(VALIDADE_FUTURA.into());
    input.itens = vec![];
    assert!(matches!(orcamentos::criar(&conn, &input, "x").unwrap_err(), AppError::Validation(_)));
}

#[test]
fn envio_e_aprovacao_registram_forma_e_quando() {
    let conn = cenario();
    let id = novo_orcamento(&conn);
    let enviado = orcamentos::enviar(&conn, id, "2026-11-22T10:05:00+00:00").unwrap();
    assert_eq!(enviado.status, Status::AguardandoAprovacao);
    assert!(enviado.enviado_em.is_some());

    let aprovado = orcamentos::aprovar(&conn, id, "WhatsApp", Some("  mandou foto da aprovação  "), "2026-11-22T11:00:00+00:00").unwrap();
    assert_eq!(aprovado.status, Status::Aprovado);
    assert_eq!(aprovado.aprovacao_forma.as_deref(), Some("WhatsApp"));
    assert_eq!(aprovado.aprovacao_obs.as_deref(), Some("mandou foto da aprovação"));
    assert!(aprovado.aprovado_em.is_some());

    // Depois de aprovado não aprova de novo nem recusa.
    assert!(matches!(orcamentos::aprovar(&conn, id, "telefone", None, "x").unwrap_err(), AppError::Conflict(_)));
    assert!(matches!(orcamentos::recusar(&conn, id, "x").unwrap_err(), AppError::Conflict(_)));
}

#[test]
fn aprovar_sem_enviar_e_recusado() {
    let conn = cenario();
    let id = novo_orcamento(&conn);
    assert!(matches!(orcamentos::aprovar(&conn, id, "presencial", None, "x").unwrap_err(), AppError::Conflict(_)));
    assert!(matches!(orcamentos::recusar(&conn, id, "x").unwrap_err(), AppError::Conflict(_)));
    // Rascunho pode ser cancelado.
    assert_eq!(orcamentos::cancelar(&conn, id, "x").unwrap().status, Status::Cancelado);
}

#[test]
fn forma_de_aprovacao_so_aceita_as_opcoes_e_outro_exige_observacao() {
    let conn = cenario();
    let id = novo_orcamento(&conn);
    orcamentos::enviar(&conn, id, "x").unwrap();
    assert!(matches!(orcamentos::aprovar(&conn, id, "email", None, "x").unwrap_err(), AppError::Validation(_)));
    assert!(matches!(orcamentos::aprovar(&conn, id, "outro", Some("   "), "x").unwrap_err(), AppError::Validation(_)));
    // Nada mudou com as tentativas inválidas.
    assert_eq!(orcamentos::buscar(&conn, id).unwrap().status, Status::AguardandoAprovacao);
    let ok = orcamentos::aprovar(&conn, id, "outro", Some("pelo filho do cliente"), "x").unwrap();
    assert_eq!(ok.aprovacao_forma.as_deref(), Some("outro"));
}

#[test]
fn orcamento_vencido_nao_pode_ser_aprovado() {
    let conn = cenario();
    let id = novo_orcamento(&conn);
    orcamentos::enviar(&conn, id, "x").unwrap();
    conn.execute("UPDATE orcamentos SET validade = '2000-01-01' WHERE id = ?1", [id]).unwrap();
    match orcamentos::aprovar(&conn, id, "presencial", None, "x").unwrap_err() {
        AppError::Conflict(msg) => assert!(msg.contains("venceu"), "{msg}"),
        outro => panic!("erro inesperado: {outro:?}"),
    }
}

#[test]
fn valor_congelado_depois_da_aprovacao() {
    let conn = cenario();
    let id = novo_orcamento(&conn);
    aprovar_orcamento(&conn, id);
    // O preço do cadastro muda depois: o orçamento aprovado não muda.
    conn.execute("UPDATE itens SET preco_venda = 999 WHERE id = 1", []).unwrap();
    let orc = orcamentos::buscar(&conn, id).unwrap();
    assert_eq!(orc.valor_total_centavos, 11000);
    assert_eq!(orc.itens[0].preco_unitario_centavos, 3000);
}

#[test]
fn converter_em_os_sem_redigitar_e_com_vinculo_nos_dois_sentidos() {
    let conn = cenario();
    let id = novo_orcamento(&conn);
    aprovar_orcamento(&conn, id);
    conn.execute("UPDATE itens SET preco_venda = 999 WHERE id = 1", []).unwrap();

    let os = orcamentos::converter_os(&conn, id, None, "2026-11-22T12:00:00+00:00").unwrap();
    assert_eq!(os.tipo, "OS");
    assert_eq!(os.status, ordens::Status::Criada);
    assert_eq!(os.veiculo_id, Some(1));
    assert_eq!(os.cliente_id, Some(1));
    assert_eq!(os.funcionario_id, 1);
    assert_eq!(os.itens.len(), 2);
    assert_eq!(os.itens[0].item_id, 1);
    assert_eq!(os.itens[0].quantidade, 2);
    // Preço do orçamento aprovado (30,00), não o novo (999).
    assert!((os.itens[0].preco_unitario - 30.0).abs() < 1e-9);
    assert!((os.valor_total - 110.0).abs() < 1e-9);

    let orc = orcamentos::buscar(&conn, id).unwrap();
    assert_eq!(orc.ordem_id, Some(os.id));
    let orc_da_os: Option<i64> = conn
        .query_row("SELECT orcamento_id FROM ordens_servico WHERE id = ?1", [os.id], |r| r.get(0))
        .unwrap();
    assert_eq!(orc_da_os, Some(id));

    // Não converte duas vezes.
    assert!(matches!(orcamentos::converter_os(&conn, id, None, "x").unwrap_err(), AppError::Conflict(_)));
}

#[test]
fn so_converte_orcamento_aprovado() {
    let conn = cenario();
    let id = novo_orcamento(&conn);
    assert!(matches!(orcamentos::converter_os(&conn, id, None, "x").unwrap_err(), AppError::Conflict(_)));
    orcamentos::enviar(&conn, id, "x").unwrap();
    assert!(matches!(orcamentos::converter_os(&conn, id, None, "x").unwrap_err(), AppError::Conflict(_)));
}

#[test]
fn aditivo_so_entra_na_os_depois_de_aprovado() {
    let conn = cenario();
    let os = ordens::criar(
        &conn,
        &OrdemInput { veiculo_id: Some(1), funcionario_id: Some(1), descricao: None, itens: vec![linha(2, 1)] },
    )
    .unwrap();
    assert!((os.valor_total - 50.0).abs() < 1e-9);

    let adicional = orcamentos::adicional(
        &conn,
        &AditivoInput { os_id: Some(os.id), observacoes: Some("Pastilha gasta".into()), itens: vec![linha(1, 1)] },
        "x",
    )
    .unwrap();
    assert_eq!(adicional.tipo, "ADITIVO");
    assert_eq!(adicional.status, Status::Rascunho);
    assert_eq!(adicional.ordem_id, Some(os.id));
    assert_eq!(adicional.valor_total_centavos, 3000);

    // Enquanto não aprovado, a OS continua igual.
    let antes = ordens::buscar(&conn, os.id).unwrap();
    assert_eq!(antes.itens.len(), 1);
    orcamentos::enviar(&conn, adicional.id, "x").unwrap();
    let ainda = ordens::buscar(&conn, os.id).unwrap();
    assert_eq!(ainda.itens.len(), 1);
    assert!((ainda.valor_total - 50.0).abs() < 1e-9);

    // Aprovado: entra na OS e o total da OS é recalculado.
    orcamentos::aprovar(&conn, adicional.id, "presencial", None, "x").unwrap();
    let depois = ordens::buscar(&conn, os.id).unwrap();
    assert_eq!(depois.itens.len(), 2);
    assert!((depois.valor_total - 80.0).abs() < 1e-9);
}

#[test]
fn aditivo_nao_vale_se_a_os_ja_foi_cancelada() {
    let conn = cenario();
    let os = ordens::criar(
        &conn,
        &OrdemInput { veiculo_id: Some(1), funcionario_id: Some(1), descricao: None, itens: vec![linha(2, 1)] },
    )
    .unwrap();
    let adicional = orcamentos::adicional(&conn, &AditivoInput { os_id: Some(os.id), observacoes: None, itens: vec![linha(1, 1)] }, "x").unwrap();
    orcamentos::enviar(&conn, adicional.id, "x").unwrap();
    conn.execute("UPDATE ordens_servico SET status = 'CANCELADA' WHERE id = ?1", [os.id]).unwrap();
    assert!(matches!(orcamentos::aprovar(&conn, adicional.id, "telefone", None, "x").unwrap_err(), AppError::Conflict(_)));
    // Nova adição também não: a OS não aceita mais peças.
    assert!(matches!(
        orcamentos::adicional(&conn, &AditivoInput { os_id: Some(os.id), observacoes: None, itens: vec![linha(1, 1)] }, "x").unwrap_err(),
        AppError::Conflict(_)
    ));
}

#[test]
fn comandos_despacham_o_fluxo_completo() {
    let conn = cenario();
    let ctx = Contexto { usuario: "Carlos".into(), perfil: "funcionario".into(), computador: "PC1".into() };
    assert!(comandos::e_escrita("orcamento_aprovar"));
    assert!(!comandos::e_escrita("orcamento_listar"));

    let criado = comandos::executar(
        &conn,
        "orcamento_criar",
        &json!({ "dados": {
            "cliente_id": 1, "veiculo_id": 1, "funcionario_id": 1,
            "validade": VALIDADE_FUTURA, "itens": [{ "item_id": 1, "quantidade": 2 }]
        } }),
        &ctx,
    )
    .unwrap();
    let id = criado["id"].as_i64().unwrap();
    assert_eq!(criado["status"], "RASCUNHO");
    assert_eq!(criado["valor_total_centavos"], 6000);

    comandos::executar(&conn, "orcamento_enviar", &json!({ "id": id }), &ctx).unwrap();
    let aprovado = comandos::executar(
        &conn,
        "orcamento_aprovar",
        &json!({ "id": id, "forma": "pix", "observacao": null }),
        &ctx,
    );
    assert!(aprovado.is_err(), "forma inválida não pode ser aceita");
    let aprovado = comandos::executar(&conn, "orcamento_aprovar", &json!({ "id": id, "forma": "telefone" }), &ctx).unwrap();
    assert_eq!(aprovado["status"], "APROVADO");

    let os = comandos::executar(&conn, "orcamento_converter_os", &json!({ "id": id }), &ctx).unwrap();
    assert_eq!(os["tipo"], "OS");
    assert_eq!(os["veiculo_id"], 1);
}
