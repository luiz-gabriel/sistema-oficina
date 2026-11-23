// Histórico do veículo: quilometragem por atendimento, ordem do histórico e garantia.
use oficina_core::ordens::{self, LinhaInput, OrdemInput};
use oficina_core::veiculos::{self, DadosAtendimento};
use oficina_core::{db, error::AppError};
use rusqlite::Connection;

fn cenario() -> Connection {
    let conn = db::open_in_memory().unwrap();
    conn.execute_batch(
        "INSERT INTO clientes (id, nome) VALUES (1, 'Ana');
         INSERT INTO veiculos (id, cliente_id, placa, modelo) VALUES (1, 1, 'ABC1D23', 'Polo');
         INSERT INTO funcionarios (id, nome) VALUES (1, 'Carlos');
         INSERT INTO itens (id, nome, tipo, preco_venda, quantidade) VALUES (1, 'Filtro', 'peca', 30, 5);
         INSERT INTO itens (id, nome, tipo, preco_venda) VALUES (2, 'Troca de óleo', 'servico', 50);",
    )
    .unwrap();
    conn
}

fn nova_os(conn: &Connection, data: &str) -> i64 {
    let input = OrdemInput {
        veiculo_id: Some(1),
        funcionario_id: Some(1),
        descricao: Some("Revisão".into()),
        itens: vec![LinhaInput { item_id: Some(2), quantidade: Some(1) }],
    };
    let id = ordens::criar(conn, &input).unwrap().id;
    conn.execute("UPDATE ordens_servico SET data_entrada = ?1 WHERE id = ?2", rusqlite::params![data, id]).unwrap();
    id
}

fn km(entrada: Option<i64>) -> DadosAtendimento {
    DadosAtendimento { km_entrada: entrada, ..Default::default() }
}

#[test]
fn quilometragem_antiga_nao_muda_ao_cadastrar_nova() {
    let conn = cenario();
    let primeira = nova_os(&conn, "2026-03-01T10:00:00+00:00");
    let segunda = nova_os(&conn, "2026-06-01T10:00:00+00:00");

    veiculos::salvar_dados_ordem(&conn, primeira, &km(Some(50_000))).unwrap();
    veiculos::salvar_dados_ordem(&conn, segunda, &km(Some(52_000))).unwrap();
    // A entrega da segunda não mexe na entrada da primeira.
    veiculos::salvar_dados_ordem(&conn, segunda, &DadosAtendimento { km_entrega: Some(52_400), ..Default::default() }).unwrap();

    let h = veiculos::historico(&conn, 1).unwrap();
    let por_id = |id: i64| h.entradas.iter().find(|e| e.ordem.id == id).unwrap();
    assert_eq!(por_id(primeira).km_entrada, Some(50_000));
    assert_eq!(por_id(primeira).km_entrega, None);
    assert_eq!(por_id(segunda).km_entrada, Some(52_000));
    assert_eq!(por_id(segunda).km_entrega, Some(52_400));
    assert_eq!(h.km_mais_recente, Some(52_400));
}

#[test]
fn entrega_menor_que_entrada_e_recusada_e_nao_grava() {
    let conn = cenario();
    let id = nova_os(&conn, "2026-03-01T10:00:00+00:00");
    veiculos::salvar_dados_ordem(&conn, id, &km(Some(50_000))).unwrap();
    let err = veiculos::salvar_dados_ordem(&conn, id, &DadosAtendimento { km_entrega: Some(49_000), ..Default::default() }).unwrap_err();
    match err {
        AppError::Validation(erros) => assert!(erros.iter().any(|e| e.field == "km_entrega")),
        outro => panic!("esperava validação, veio {outro:?}"),
    }
    let h = veiculos::historico(&conn, 1).unwrap();
    assert_eq!(h.entradas[0].km_entrega, None);
}

#[test]
fn historico_vai_do_mais_recente_para_o_mais_antigo() {
    let conn = cenario();
    let antiga = nova_os(&conn, "2025-01-10T10:00:00+00:00");
    let recente = nova_os(&conn, "2026-05-10T10:00:00+00:00");
    let meio = nova_os(&conn, "2025-09-10T10:00:00+00:00");
    let h = veiculos::historico(&conn, 1).unwrap();
    let ids: Vec<i64> = h.entradas.iter().map(|e| e.ordem.id).collect();
    assert_eq!(ids, vec![recente, meio, antiga]);
    // Peças e serviços aparecem em cada entrada.
    assert_eq!(h.entradas[0].ordem.itens.len(), 1);
    assert_eq!(h.entradas[0].ordem.itens[0].item_nome, "Troca de óleo");
}

#[test]
fn garantia_conta_a_partir_da_entrega_e_revisao_aparece() {
    let conn = cenario();
    let id = nova_os(&conn, "2026-10-01T10:00:00+00:00");
    ordens::alterar_status(&conn, id, ordens::Status::EmAndamento).unwrap();
    ordens::concluir_com_pagamento(&conn, id, "pix").unwrap();
    conn.execute("UPDATE ordens_servico SET data_conclusao = '2026-11-10T12:00:00+00:00' WHERE id = ?1", [id]).unwrap();
    veiculos::salvar_dados_ordem(&conn, id, &DadosAtendimento {
        garantia_dias: Some(90),
        revisao: Some(true),
        prazo_previsto: Some("2026-11-12".into()),
        ..Default::default()
    })
    .unwrap();
    let h = veiculos::historico(&conn, 1).unwrap();
    let e = &h.entradas[0];
    // 10/11/2026 + 90 dias = 08/02/2027.
    assert_eq!(e.garantia_ate.as_deref(), Some("2027-02-08"));
    assert!(e.revisao);
    assert_eq!(e.prazo_previsto.as_deref(), Some("2026-11-12"));
}

#[test]
fn venda_de_balcao_nao_recebe_quilometragem() {
    let conn = cenario();
    let id = nova_os(&conn, "2026-03-01T10:00:00+00:00");
    conn.execute("UPDATE ordens_servico SET tipo = 'BALCAO', veiculo_id = NULL WHERE id = ?1", [id]).unwrap();
    let err = veiculos::salvar_dados_ordem(&conn, id, &km(Some(1000))).unwrap_err();
    assert!(matches!(err, AppError::Conflict(_)));
}
