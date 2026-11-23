//! Fornecedores, compras, entrada de estoque, custo médio e histórico.
use oficina_core::comandos::{e_escrita, executar, tabela_do_comando, Contexto};
use oficina_core::compras::{self, custo_medio_novo, CompraInput, LinhaInput};
use oficina_core::db;
use oficina_core::error::AppError;
use oficina_core::fornecedores::{self, FornecedorInput};
use rusqlite::Connection;
use serde_json::json;

fn admin() -> Contexto {
    Contexto { usuario: "Carlos".into(), perfil: "administrador".into(), computador: "PC1".into() }
}

/// Banco com três itens: 1 = peça sem estoque, 2 = peça com 3 unidades a R$ 10,00, 3 = serviço.
fn cenario() -> Connection {
    let conn = db::open_in_memory().unwrap();
    executar(&conn, "itens_criar", &json!({"dados": {"nome": "Filtro de óleo", "tipo": "peca", "preco_venda": 40.0}}), &admin()).unwrap();
    executar(&conn, "itens_criar", &json!({"dados": {"nome": "Pastilha", "tipo": "peca", "preco_venda": 120.0, "quantidade": 3, "preco_custo": 10.0}}), &admin()).unwrap();
    executar(&conn, "itens_criar", &json!({"dados": {"nome": "Troca de óleo", "tipo": "servico", "preco_venda": 50.0}}), &admin()).unwrap();
    conn
}

fn fornecedor(conn: &Connection, documento: &str) -> i64 {
    fornecedores::criar(conn, &FornecedorInput {
        nome: Some("Distribuidora Sul".into()),
        documento: Some(documento.into()),
        ..Default::default()
    })
    .unwrap()
    .id
}

fn linha(item: i64, quantidade: i64, custo: i64) -> LinhaInput {
    LinhaInput { item_id: Some(item), quantidade: Some(quantidade), custo_unitario_centavos: Some(custo) }
}

fn compra(fornecedor: i64, pagamento: &str, itens: Vec<LinhaInput>) -> CompraInput {
    CompraInput {
        fornecedor_id: Some(fornecedor),
        data: Some("2026-11-10".into()),
        pagamento: Some(pagamento.into()),
        vencimento: None,
        itens,
    }
}

fn contar(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).unwrap()
}

fn estoque(conn: &Connection, item: i64) -> (i64, f64) {
    conn.query_row("SELECT quantidade, preco_custo FROM itens WHERE id = ?1", [item], |r| Ok((r.get(0)?, r.get(1)?))).unwrap()
}

fn erro_de_campo(r: Result<impl std::fmt::Debug, AppError>, campo: &str) -> bool {
    match r {
        Err(AppError::Validation(erros)) => erros.iter().any(|e| e.field == campo),
        _ => false,
    }
}

// ---- CPF e CNPJ ----

#[test]
fn cpf_valido_e_invalido() {
    assert!(fornecedores::cpf_valido("52998224725"));
    assert_eq!(fornecedores::normalizar_documento("529.982.247-25").unwrap(), "52998224725");
    assert!(fornecedores::normalizar_documento("529.982.247-26").is_err(), "dígito verificador errado");
    assert!(fornecedores::normalizar_documento("111.111.111-11").is_err(), "sequência repetida");
    assert!(fornecedores::normalizar_documento("123.456.789").is_err(), "tamanho errado");
    assert!(fornecedores::normalizar_documento("529.982.247-2a").is_err(), "letra no documento");
}

#[test]
fn cnpj_valido_e_invalido() {
    assert_eq!(fornecedores::normalizar_documento("11.222.333/0001-81").unwrap(), "11222333000181");
    assert!(fornecedores::normalizar_documento("11.222.333/0001-82").is_err(), "dígito verificador errado");
    assert!(fornecedores::normalizar_documento("00.000.000/0000-00").is_err(), "sequência repetida");
}

#[test]
fn cadastro_recusa_documento_invalido_e_duplicado() {
    let conn = cenario();
    let invalido = fornecedores::criar(&conn, &FornecedorInput {
        nome: Some("X".into()),
        documento: Some("529.982.247-26".into()),
        ..Default::default()
    });
    assert!(erro_de_campo(invalido, "documento"));

    fornecedor(&conn, "529.982.247-25");
    // Mesmo CPF, escrito sem pontuação: é o mesmo documento e não pode duplicar.
    let duplicado = fornecedores::criar(&conn, &FornecedorInput {
        nome: Some("Outro nome".into()),
        documento: Some("52998224725".into()),
        ..Default::default()
    });
    assert!(erro_de_campo(duplicado, "documento"));
    assert_eq!(contar(&conn, "SELECT COUNT(*) FROM fornecedores"), 1);
}

#[test]
fn cadastro_listagem_edicao_e_busca() {
    let conn = cenario();
    let id = fornecedor(&conn, "11.222.333/0001-81");
    fornecedores::criar(&conn, &FornecedorInput {
        nome: Some("Auto Peças Norte".into()),
        documento: Some("52998224725".into()),
        email: Some("contato@norte.com.br".into()),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(fornecedores::listar(&conn).unwrap().len(), 2);

    let editado = fornecedores::atualizar(&conn, id, &FornecedorInput {
        nome: Some("Distribuidora Sul Ltda".into()),
        documento: Some("11.222.333/0001-81".into()),
        telefone: Some("(11) 3333-4444".into()),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(editado.nome, "Distribuidora Sul Ltda");
    assert_eq!(editado.documento, "11222333000181");

    assert_eq!(fornecedores::buscar_por_termo(&conn, "sul").unwrap().len(), 1, "busca por nome");
    assert_eq!(fornecedores::buscar_por_termo(&conn, "529.982").unwrap().len(), 1, "busca por parte do CPF");
    assert_eq!(fornecedores::buscar_por_termo(&conn, "11.222.333/0001-81").unwrap()[0].id, id, "busca por CNPJ");
    assert!(fornecedores::buscar_por_termo(&conn, "inexistente").unwrap().is_empty());
    assert_eq!(fornecedores::buscar_por_termo(&conn, "  ").unwrap().len(), 2, "termo vazio lista tudo");
}

// ---- Compra, entrada de estoque e conta a pagar ----

#[test]
fn compra_a_vista_da_entrada_no_estoque_e_marca_como_paga() {
    let conn = cenario();
    let f = fornecedor(&conn, "52998224725");
    let c = compras::registrar(&conn, &compra(f, "pix", vec![linha(1, 5, 1000), linha(2, 2, 1600)]), "Carlos").unwrap();

    assert_eq!(c.total_centavos, 5 * 1000 + 2 * 1600);
    assert!(c.paga);
    assert!(c.vencimento.is_none());
    assert_eq!(estoque(&conn, 1).0, 5);
    assert_eq!(estoque(&conn, 2).0, 5);
    assert_eq!(contar(&conn, "SELECT COUNT(*) FROM compra_itens"), 2);
    assert_eq!(contar(&conn, "SELECT COUNT(*) FROM contas_pagar WHERE compra_id IS NOT NULL"), 0, "compra paga não gera conta");
}

#[test]
fn validacoes_da_compra_nao_gravam_nada() {
    let conn = cenario();
    let f = fornecedor(&conn, "52998224725");

    assert!(erro_de_campo(compras::registrar(&conn, &compra(f, "pix", vec![linha(1, 0, 1000)]), "Carlos"), "itens"), "quantidade zero");
    assert!(erro_de_campo(compras::registrar(&conn, &compra(f, "pix", vec![linha(3, 1, 1000)]), "Carlos"), "itens"), "serviço não entra em compra");
    assert!(erro_de_campo(compras::registrar(&conn, &compra(f, "pix", vec![]), "Carlos"), "itens"), "sem itens");
    assert!(erro_de_campo(compras::registrar(&conn, &compra(f, "cheque", vec![linha(1, 1, 1000)]), "Carlos"), "pagamento"), "forma desconhecida");
    assert!(erro_de_campo(compras::registrar(&conn, &compra(999, "pix", vec![linha(1, 1, 1000)]), "Carlos"), "fornecedor_id"), "fornecedor inexistente");
    assert!(erro_de_campo(compras::registrar(&conn, &compra(f, "pix", vec![linha(99, 1, 1000)]), "Carlos"), "itens"), "peça inexistente");

    assert_eq!(contar(&conn, "SELECT COUNT(*) FROM compras"), 0);
    assert_eq!(estoque(&conn, 1).0, 0);
}

// ---- Custo médio ----

#[test]
fn custo_medio_ponderado_em_centavos_com_arredondamento_para_baixo() {
    // Regra: (estoque × custo atual + qtd × custo da compra) / (estoque + qtd), divisão inteira.
    assert_eq!(custo_medio_novo(3, 1000, 2, 1600), 1240, "(3000 + 3200) / 5");
    assert_eq!(custo_medio_novo(2, 1000, 1, 1001), 1000, "3001 / 3 = 1000,33 arredonda para 1000");
    // Estoque zero: o custo antigo não conta, vale só o custo da compra.
    assert_eq!(custo_medio_novo(0, 999, 4, 250), 250);
    // Estoque negativo no banco é tratado como zero.
    assert_eq!(custo_medio_novo(-2, 999, 4, 250), 250);
}

#[test]
fn custo_medio_e_gravado_em_itens_preco_custo() {
    let conn = cenario();
    let f = fornecedor(&conn, "52998224725");

    // Item 2 tem 3 unidades a R$ 10,00. Entram 2 a R$ 16,00: custo médio R$ 12,40.
    compras::registrar(&conn, &compra(f, "dinheiro", vec![linha(2, 2, 1600)]), "Carlos").unwrap();
    assert_eq!(estoque(&conn, 2), (5, 12.4));

    // Item 1 começa sem estoque: o custo da primeira compra vira o custo do item.
    compras::registrar(&conn, &compra(f, "dinheiro", vec![linha(1, 4, 250)]), "Carlos").unwrap();
    assert_eq!(estoque(&conn, 1), (4, 2.5));
}

// ---- Atomicidade ----

#[test]
fn falha_no_meio_da_compra_desfaz_tudo() {
    let conn = cenario();
    let f = fornecedor(&conn, "52998224725");
    // Falha simulada: a segunda atualização de estoque é recusada pelo banco.
    conn.execute_batch(
        "CREATE TRIGGER falha_estoque BEFORE UPDATE OF quantidade ON itens
         WHEN NEW.id = 2 BEGIN SELECT RAISE(ABORT, 'falha simulada'); END;",
    )
    .unwrap();

    let r = compras::registrar(&conn, &compra(f, "pix", vec![linha(1, 5, 1000), linha(2, 2, 1600)]), "Carlos");
    assert!(r.is_err());

    // A primeira linha já tinha sido gravada antes da falha: nada pode ficar pela metade.
    assert_eq!(estoque(&conn, 1), (0, 0.0), "estoque do item 1 não pode mudar");
    assert_eq!(estoque(&conn, 2), (3, 10.0), "item 2 não pode mudar");
    assert_eq!(contar(&conn, "SELECT COUNT(*) FROM compras"), 0);
    assert_eq!(contar(&conn, "SELECT COUNT(*) FROM compra_itens"), 0);
}

#[test]
fn falha_ao_gerar_conta_a_prazo_desfaz_estoque_e_compra() {
    let conn = cenario();
    let f = fornecedor(&conn, "52998224725");
    conn.execute_batch(
        "CREATE TRIGGER falha_conta BEFORE INSERT ON contas_pagar
         BEGIN SELECT RAISE(ABORT, 'falha simulada'); END;",
    )
    .unwrap();

    let mut c = compra(f, "a pagar", vec![linha(1, 5, 1000)]);
    c.vencimento = Some("2026-12-10".into());
    assert!(compras::registrar(&conn, &c, "Carlos").is_err());

    assert_eq!(estoque(&conn, 1), (0, 0.0), "entrada de estoque foi desfeita junto com a compra");
    assert_eq!(contar(&conn, "SELECT COUNT(*) FROM compras"), 0);
    assert_eq!(contar(&conn, "SELECT COUNT(*) FROM compra_itens"), 0);
}

// ---- Compra a prazo ----

#[test]
fn compra_a_prazo_gera_uma_unica_conta() {
    let conn = cenario();
    let f = fornecedor(&conn, "52998224725");
    let mut c = compra(f, "a pagar", vec![linha(1, 5, 1000), linha(2, 1, 2000)]);
    c.vencimento = Some("2026-12-10".into());
    let criada = compras::registrar(&conn, &c, "Carlos").unwrap();

    assert!(!criada.paga);
    assert_eq!(criada.vencimento.as_deref(), Some("2026-12-10"));
    assert_eq!(estoque(&conn, 1).0, 5, "a prazo também dá entrada no estoque");

    // A conta a pagar fica em contas_pagar, com compra_id, descrição, fornecedor e data da compra.
    let (descricao, fornecedor, valor, vencimento, data): (String, String, i64, String, String) = conn
        .query_row(
            "SELECT descricao, fornecedor, valor_centavos, vencimento, data FROM contas_pagar WHERE compra_id = ?1",
            [criada.id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .unwrap();
    assert_eq!(descricao, format!("Compra #{}", criada.id));
    assert_eq!(fornecedor, "Distribuidora Sul");
    assert_eq!((valor, vencimento.as_str(), data.as_str()), (criada.total_centavos, "2026-12-10", "2026-11-10"));

    // Uma compra gera uma única conta (a inserção é feita só por `registrar`).
    assert_eq!(contar(&conn, &format!("SELECT COUNT(*) FROM contas_pagar WHERE compra_id = {}", criada.id)), 1);
}

#[test]
fn compra_a_prazo_exige_vencimento() {
    let conn = cenario();
    let f = fornecedor(&conn, "52998224725");
    let r = compras::registrar(&conn, &compra(f, "a pagar", vec![linha(1, 1, 1000)]), "Carlos");
    assert!(erro_de_campo(r, "vencimento"));
    assert_eq!(contar(&conn, "SELECT COUNT(*) FROM compras"), 0);
}

// ---- Histórico ----

#[test]
fn historico_por_fornecedor_e_por_item() {
    let conn = cenario();
    let f1 = fornecedor(&conn, "52998224725");
    let f2 = fornecedor(&conn, "11.222.333/0001-81");

    let mut a_prazo = compra(f1, "a pagar", vec![linha(1, 3, 1200)]);
    a_prazo.data = Some("2026-11-15".into());
    a_prazo.vencimento = Some("2026-12-15".into());
    compras::registrar(&conn, &a_prazo, "Carlos").unwrap();

    let mut antiga = compra(f1, "pix", vec![linha(1, 2, 1000), linha(2, 1, 1500)]);
    antiga.data = Some("2026-11-01".into());
    compras::registrar(&conn, &antiga, "Carlos").unwrap();

    let mut de_outro = compra(f2, "pix", vec![linha(1, 1, 900)]);
    de_outro.data = Some("2026-11-10".into());
    compras::registrar(&conn, &de_outro, "Carlos").unwrap();

    let do_f1 = compras::historico_fornecedor(&conn, f1).unwrap();
    assert_eq!(do_f1.len(), 2);
    assert_eq!(do_f1[0].data, "2026-11-15", "mais recente primeiro");
    assert_eq!(do_f1[0].itens.len(), 1);
    assert_eq!(do_f1[1].itens.len(), 2);
    assert_eq!(do_f1[1].itens[0].item_nome, "Filtro de óleo");

    let do_item = compras::historico_item(&conn, 1).unwrap();
    assert_eq!(do_item.len(), 3);
    assert_eq!(do_item.iter().map(|h| h.fornecedor_id).collect::<Vec<_>>(), vec![f1, f2, f1]);
    assert_eq!(do_item[0].custo_unitario_centavos, 1200);

    assert!(matches!(compras::historico_fornecedor(&conn, 999), Err(AppError::NotFound(_))));
    assert_eq!(compras::listar(&conn).unwrap().len(), 3);
}

// ---- Comandos e sincronização ----

#[test]
fn comandos_de_compras_pelo_despacho_unico() {
    let conn = cenario();
    let f = executar(&conn, "fornecedores_criar", &json!({"dados": {"nome": "Sul", "documento": "529.982.247-25"}}), &admin())
        .unwrap()["id"]
        .as_i64()
        .unwrap();
    executar(&conn, "fornecedores_atualizar", &json!({"id": f, "dados": {"nome": "Sul Ltda", "documento": "52998224725"}}), &admin()).unwrap();

    let busca = executar(&conn, "fornecedores_buscar", &json!({"termo": "sul"}), &admin()).unwrap();
    assert_eq!(busca.as_array().unwrap().len(), 1);

    let registrada = executar(
        &conn,
        "compras_registrar",
        &json!({"dados": {"fornecedor_id": f, "pagamento": "pix", "itens": [{"item_id": 1, "quantidade": 5, "custo_unitario_centavos": 1000}]}}),
        &admin(),
    )
    .unwrap();
    assert_eq!(registrada["total_centavos"], 5000);

    let historico = executar(&conn, "compras_historico_fornecedor", &json!({"fornecedor_id": f}), &admin()).unwrap();
    assert_eq!(historico.as_array().unwrap().len(), 1);
    let por_item = executar(&conn, "compras_historico_item", &json!({"item_id": 1}), &admin()).unwrap();
    assert_eq!(por_item.as_array().unwrap().len(), 1);
    assert_eq!(executar(&conn, "compras_listar", &json!({}), &admin()).unwrap().as_array().unwrap().len(), 1);

    // Compra e cadastro de fornecedor entram na fila de sincronização; leituras não.
    assert!(e_escrita("compras_registrar"));
    assert!(e_escrita("fornecedores_criar"));
    assert!(!e_escrita("compras_listar"));
    assert_eq!(tabela_do_comando("compras_registrar"), Some("compras"));
    assert_eq!(tabela_do_comando("fornecedores_atualizar"), Some("fornecedores"));
}
