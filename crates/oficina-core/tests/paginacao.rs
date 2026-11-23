//! Listas paginadas: cada bloco traz registros diferentes, sem repetir nem pular, e as ordens trazem os seus itens.
//! Também cobre o modo WAL e a piscina de conexões de um banco de arquivo.
use oficina_core::caixa;
use oficina_core::comandos::{executar, Contexto};
use oficina_core::db;
use oficina_core::paginacao::{Pagina, TAMANHO_MAXIMO, TAMANHO_PADRAO};
use serde_json::{json, Value};
use std::collections::HashSet;

fn ctx() -> Contexto {
    Contexto { usuario: "Carlos".into(), perfil: "administrador".into(), computador: "PC1".into() }
}

fn pasta(nome: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!("oficina_pag_{nome}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    p
}

#[test]
fn blocos_cobrem_a_lista_inteira_sem_repetir_nem_pular() {
    let conn = db::open_in_memory().unwrap();
    for i in 0..250 {
        // Nomes iguais de propósito: o desempate pelo id é o que impede repetição entre blocos.
        conn.execute("INSERT INTO clientes (nome) VALUES (?1)", [format!("Mesmo nome {}", i % 5)]).unwrap();
    }
    let mut vistos = HashSet::new();
    let mut deslocamento = 0;
    loop {
        let bloco = oficina_core::clientes::listar_pagina(&conn, Pagina::nova(Some(80), Some(deslocamento))).unwrap();
        for c in &bloco {
            assert!(vistos.insert(c.id), "cliente {} apareceu em dois blocos", c.id);
        }
        if bloco.len() < 80 {
            break;
        }
        deslocamento += 80;
    }
    assert_eq!(vistos.len(), 250);
}

#[test]
fn comando_de_pagina_respeita_limite_e_padrao() {
    let conn = db::open_in_memory().unwrap();
    for i in 0..(TAMANHO_PADRAO + 30) {
        conn.execute("INSERT INTO clientes (nome) VALUES (?1)", [format!("Cliente {i:04}")]).unwrap();
    }
    let padrao = executar(&conn, "clientes_pagina", &json!({}), &ctx()).unwrap();
    assert_eq!(padrao.as_array().unwrap().len(), TAMANHO_PADRAO as usize);
    let grande = executar(&conn, "clientes_pagina", &json!({"limite": 100000}), &ctx()).unwrap();
    assert_eq!(grande.as_array().unwrap().len(), (TAMANHO_PADRAO + 30) as usize);
    assert!(TAMANHO_MAXIMO >= 1);
    let ruim = executar(&conn, "clientes_pagina", &json!({"limite": "x", "deslocamento": -4}), &ctx()).unwrap();
    assert_eq!(ruim.as_array().unwrap().len(), TAMANHO_PADRAO as usize, "valor inválido cai no padrão");
}

#[test]
fn ordens_paginadas_trazem_os_itens_de_cada_uma() {
    let conn = db::open_in_memory().unwrap();
    conn.execute_batch("INSERT INTO funcionarios (id, nome) VALUES (1, 'Carlos'); INSERT INTO clientes (id, nome) VALUES (1, 'Base');")
        .unwrap();
    for i in 1..=3 {
        conn.execute(
            "INSERT INTO itens (id, nome, tipo, codigo, preco_venda, quantidade, estoque_minimo) VALUES (?1, ?2, 'peca', ?3, 10, 100, 1)",
            rusqlite::params![i, format!("Peça {i}"), format!("C{i}")],
        )
        .unwrap();
    }
    caixa::abrir(&conn, Some(0.0), "Carlos", &chrono::Utc::now().to_rfc3339()).unwrap();
    // Venda 1: 1 peça. Venda 2: 2 peças diferentes. Assim o lote tem ordens com 1 e 2 itens.
    let ctx = ctx();
    executar(&conn, "venda_balcao_criar", &json!({"dados": {"cliente_id": 1, "funcionario_id": 1, "pagamento": "pix",
        "itens": [{"item_id": 1, "quantidade": 1}]}}), &ctx).unwrap();
    executar(&conn, "venda_balcao_criar", &json!({"dados": {"cliente_id": 1, "funcionario_id": 1, "pagamento": "pix",
        "itens": [{"item_id": 2, "quantidade": 1}, {"item_id": 3, "quantidade": 2}]}}), &ctx).unwrap();

    let pagina: Value = executar(&conn, "ordens_pagina", &json!({"limite": 10, "deslocamento": 0}), &ctx).unwrap();
    let ordens = pagina.as_array().unwrap();
    assert_eq!(ordens.len(), 2);
    // Mais novas primeiro: a venda 2 (2 itens) vem antes da venda 1 (1 item).
    assert_eq!(ordens[0]["itens"].as_array().unwrap().len(), 2);
    assert_eq!(ordens[1]["itens"].as_array().unwrap().len(), 1);
    assert_eq!(ordens[1]["itens"][0]["item_nome"], "Peça 1");

    let segunda = executar(&conn, "ordens_pagina", &json!({"limite": 1, "deslocamento": 1}), &ctx).unwrap();
    assert_eq!(segunda[0]["id"], ordens[1]["id"], "o deslocamento pula a primeira ordem");
    assert_eq!(segunda[0]["itens"].as_array().unwrap().len(), 1);
}

#[test]
fn banco_de_arquivo_fica_em_wal_e_a_piscina_entrega_conexoes() {
    let dir = pasta("wal");
    let caminho = dir.join("oficina.db");
    let conn = db::open(&caminho).unwrap();
    let modo: String = conn.query_row("PRAGMA journal_mode", [], |r| r.get(0)).unwrap();
    assert_eq!(modo, "wal");
    drop(conn);

    let piscina = db::piscina(&caminho).unwrap();
    let a = piscina.get().unwrap();
    let b = piscina.get().unwrap();
    let chaves: i64 = a.query_row("PRAGMA foreign_keys", [], |r| r.get(0)).unwrap();
    assert_eq!(chaves, 1, "chave estrangeira ligada em toda conexão da piscina");
    a.execute("INSERT INTO clientes (nome) VALUES ('X')", []).unwrap();
    let n: i64 = b.query_row("SELECT COUNT(*) FROM clientes", [], |r| r.get(0)).unwrap();
    assert_eq!(n, 1, "a leitura enxerga o que outra conexão já gravou");
    drop((a, b));
    let _ = std::fs::remove_dir_all(&dir);
}
