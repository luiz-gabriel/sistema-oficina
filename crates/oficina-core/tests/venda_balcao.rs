//! Venda de balcão: sem veículo, cliente opcional, baixa de estoque, caixa obrigatório e pagamento.
//! Inclui a migração de um banco no formato antigo (ordens com veículo obrigatório), usando arquivo temporário.
use oficina_core::caixa;
use oficina_core::comandos::{executar, Contexto};
use oficina_core::db;
use oficina_core::error::AppError;
use oficina_core::ordens::{self, OrdemInput, LinhaInput};
use rusqlite::Connection;
use serde_json::{json, Value};

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

fn ctx() -> Contexto {
    Contexto { usuario: "Carlos".into(), perfil: "administrador".into(), computador: "PC1".into() }
}

fn abrir_caixa(conn: &Connection, valor: f64) {
    executar(conn, "caixa_abrir", &json!({ "valor_inicial": valor }), &ctx()).unwrap();
}

fn venda(conn: &Connection, dados: Value) -> Result<Value, AppError> {
    executar(conn, "venda_balcao_criar", &json!({ "dados": dados }), &ctx())
}

fn conta(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).unwrap()
}

#[test]
fn venda_sem_veiculo_e_sem_cliente() {
    let conn = cenario();
    abrir_caixa(&conn, 0.0);
    let v = venda(&conn, json!({"funcionario_id": 1, "pagamento": "pix", "itens": [{"item_id": 2, "quantidade": 1}]})).unwrap();

    assert_eq!(v["tipo"], "BALCAO");
    assert!(v["veiculo_id"].is_null());
    assert!(v["cliente_id"].is_null());
    assert_eq!(v["status"], "CONCLUIDA");
    assert_eq!(v["valor_total"], 50.0);
    assert_eq!(v["pagamento"], "pix");
    assert!(v["data_conclusao"].is_string());
}

#[test]
fn venda_com_cliente_sem_veiculo() {
    let conn = cenario();
    abrir_caixa(&conn, 0.0);
    let v = venda(&conn, json!({"cliente_id": 1, "funcionario_id": 1, "pagamento": "dinheiro", "itens": [{"item_id": 2, "quantidade": 1}]})).unwrap();
    assert_eq!(v["cliente_id"], 1);
    assert_eq!(v["cliente_nome"], "Ana");
    assert!(v["veiculo_id"].is_null());
}

#[test]
fn venda_baixa_so_o_estoque_das_pecas() {
    let conn = cenario();
    abrir_caixa(&conn, 0.0);
    venda(&conn, json!({"funcionario_id": 1, "pagamento": "pix", "itens": [{"item_id": 1, "quantidade": 2}, {"item_id": 2, "quantidade": 1}]})).unwrap();

    assert_eq!(oficina_core::itens::buscar(&conn, 1).unwrap().quantidade, 3); // 5 − 2
    assert_eq!(oficina_core::itens::buscar(&conn, 2).unwrap().quantidade, 0); // serviço não tem estoque
}

#[test]
fn estoque_insuficiente_nao_grava_nada() {
    let conn = cenario();
    abrir_caixa(&conn, 0.0);
    let r = venda(&conn, json!({"funcionario_id": 1, "pagamento": "pix", "itens": [{"item_id": 1, "quantidade": 9}]}));
    assert!(matches!(r, Err(AppError::Conflict(ref m)) if m.contains("Estoque insuficiente")), "{r:?}");
    assert_eq!(oficina_core::itens::buscar(&conn, 1).unwrap().quantidade, 5);
    assert_eq!(conta(&conn, "SELECT COUNT(*) FROM ordens_servico"), 0);
    assert_eq!(conta(&conn, "SELECT COUNT(*) FROM ordem_itens"), 0);
}

#[test]
fn bloqueia_venda_sem_caixa_aberto() {
    let conn = cenario();
    let r = venda(&conn, json!({"funcionario_id": 1, "pagamento": "pix", "itens": [{"item_id": 1, "quantidade": 1}]}));
    assert!(matches!(r, Err(AppError::Conflict(ref m)) if m.contains("Abra o caixa")), "{r:?}");
    assert_eq!(conta(&conn, "SELECT COUNT(*) FROM ordens_servico"), 0);
    assert_eq!(oficina_core::itens::buscar(&conn, 1).unwrap().quantidade, 5);
}

#[test]
fn pagamento_entra_no_caixa() {
    let conn = cenario();
    abrir_caixa(&conn, 100.0);
    venda(&conn, json!({"funcionario_id": 1, "pagamento": "pix", "itens": [{"item_id": 2, "quantidade": 1}]})).unwrap();
    venda(&conn, json!({"funcionario_id": 1, "pagamento": "dinheiro", "itens": [{"item_id": 1, "quantidade": 1}]})).unwrap();

    let situacao = caixa::situacao(&conn).unwrap();
    assert_eq!(situacao.recebimentos.pix, 50.0);
    assert_eq!(situacao.recebimentos.dinheiro, 30.0);
    assert_eq!(situacao.recebimentos.atendimentos, 2);
    assert_eq!(situacao.dinheiro_esperado, 130.0); // 100 de troco + 30 em dinheiro
}

#[test]
fn pagamento_obrigatorio_e_valido() {
    let conn = cenario();
    abrir_caixa(&conn, 0.0);
    let sem = venda(&conn, json!({"funcionario_id": 1, "itens": [{"item_id": 2, "quantidade": 1}]}));
    assert!(matches!(sem, Err(AppError::Validation(_))), "{sem:?}");
    let invalido = venda(&conn, json!({"funcionario_id": 1, "pagamento": "cheque", "itens": [{"item_id": 2, "quantidade": 1}]}));
    assert!(matches!(invalido, Err(AppError::Validation(_))), "{invalido:?}");
    assert_eq!(conta(&conn, "SELECT COUNT(*) FROM ordens_servico"), 0);
}

#[test]
fn venda_exige_ao_menos_um_item() {
    let conn = cenario();
    abrir_caixa(&conn, 0.0);
    let r = venda(&conn, json!({"funcionario_id": 1, "pagamento": "pix", "itens": []}));
    assert!(matches!(r, Err(AppError::Validation(_))), "{r:?}");
}

#[test]
fn ordem_de_servico_continua_exigindo_veiculo() {
    let conn = cenario();
    let input = OrdemInput {
        veiculo_id: None,
        funcionario_id: Some(1),
        descricao: None,
        itens: vec![LinhaInput { item_id: Some(2), quantidade: Some(1) }],
    };
    assert!(matches!(ordens::criar(&conn, &input), Err(AppError::Validation(_))));
}

#[test]
fn venda_aparece_na_listagem_de_ordens_com_tipo() {
    let conn = cenario();
    abrir_caixa(&conn, 0.0);
    venda(&conn, json!({"funcionario_id": 1, "pagamento": "outro", "itens": [{"item_id": 2, "quantidade": 1}]})).unwrap();
    let lista = ordens::listar(&conn).unwrap();
    assert_eq!(lista.len(), 1);
    assert_eq!(lista[0].tipo, "BALCAO");
    assert_eq!(lista[0].veiculo_id, None);
    assert_eq!(lista[0].itens.len(), 1);
}

#[test]
fn check_impede_balcao_com_veiculo_e_os_sem_veiculo() {
    let conn = cenario();
    let r = conn.execute(
        "INSERT INTO ordens_servico (tipo, veiculo_id, funcionario_id, status, data_entrada) VALUES ('BALCAO', 1, 1, 'CRIADA', 'x')",
        [],
    );
    assert!(r.is_err());
    let r = conn.execute(
        "INSERT INTO ordens_servico (tipo, veiculo_id, funcionario_id, status, data_entrada) VALUES ('OS', NULL, 1, 'CRIADA', 'x')",
        [],
    );
    assert!(r.is_err());
}

/// Banco criado pela versão antiga (veículo obrigatório, sem tipo/cliente_id), com OS reais e um id apagado.
#[test]
fn migra_banco_antigo_preservando_as_ordens() {
    let caminho = std::env::temp_dir().join(format!("oficina_migra_venda_{}.db", std::process::id()));
    let _ = std::fs::remove_file(&caminho);
    {
        let antigo = Connection::open(&caminho).unwrap();
        antigo
            .execute_batch(
                "PRAGMA foreign_keys = ON;
                 CREATE TABLE clientes (id INTEGER PRIMARY KEY AUTOINCREMENT, nome TEXT NOT NULL, telefone TEXT, email TEXT, cpf TEXT, endereco TEXT, observacoes TEXT);
                 CREATE TABLE funcionarios (id INTEGER PRIMARY KEY AUTOINCREMENT, nome TEXT NOT NULL, cargo TEXT, telefone TEXT);
                 CREATE TABLE veiculos (id INTEGER PRIMARY KEY AUTOINCREMENT, cliente_id INTEGER NOT NULL REFERENCES clientes(id), placa TEXT NOT NULL UNIQUE, marca TEXT, modelo TEXT NOT NULL, ano INTEGER);
                 CREATE TABLE itens (id INTEGER PRIMARY KEY AUTOINCREMENT, nome TEXT NOT NULL, tipo TEXT NOT NULL CHECK (tipo IN ('peca', 'servico')), codigo TEXT, preco_custo REAL NOT NULL DEFAULT 0, preco_venda REAL NOT NULL, quantidade INTEGER NOT NULL DEFAULT 0, estoque_minimo INTEGER NOT NULL DEFAULT 0);
                 CREATE TABLE ordens_servico (
                   id INTEGER PRIMARY KEY AUTOINCREMENT,
                   veiculo_id INTEGER NOT NULL REFERENCES veiculos(id),
                   funcionario_id INTEGER NOT NULL REFERENCES funcionarios(id),
                   descricao TEXT,
                   status TEXT NOT NULL CHECK (status IN ('CRIADA', 'EM_ANDAMENTO', 'CONCLUIDA', 'CANCELADA')),
                   data_entrada TEXT NOT NULL, data_conclusao TEXT, valor_total REAL NOT NULL DEFAULT 0, pagamento TEXT);
                 CREATE TABLE ordem_itens (
                   id INTEGER PRIMARY KEY AUTOINCREMENT,
                   os_id INTEGER NOT NULL REFERENCES ordens_servico(id) ON DELETE CASCADE,
                   item_id INTEGER NOT NULL REFERENCES itens(id),
                   quantidade INTEGER NOT NULL CHECK (quantidade > 0), preco_unitario REAL NOT NULL, subtotal REAL NOT NULL);
                 INSERT INTO clientes (id, nome) VALUES (1, 'Ana');
                 INSERT INTO funcionarios (id, nome) VALUES (1, 'Carlos');
                 INSERT INTO veiculos (id, cliente_id, placa, modelo) VALUES (1, 1, 'ABC1D23', 'Polo');
                 INSERT INTO itens (id, nome, tipo, preco_venda, quantidade) VALUES (1, 'Filtro', 'peca', 30, 5);
                 INSERT INTO ordens_servico (id, veiculo_id, funcionario_id, status, data_entrada, data_conclusao, valor_total, pagamento) VALUES
                   (1, 1, 1, 'CONCLUIDA', '2026-01-01T10:00:00+00:00', '2026-01-01T12:00:00+00:00', 30, 'pix'),
                   (2, 1, 1, 'CRIADA', '2026-01-02T10:00:00+00:00', NULL, 0, NULL),
                   (3, 1, 1, 'CRIADA', '2026-01-03T10:00:00+00:00', NULL, 0, NULL);
                 INSERT INTO ordem_itens (os_id, item_id, quantidade, preco_unitario, subtotal) VALUES (1, 1, 1, 30, 30);
                 DELETE FROM ordens_servico WHERE id = 3;",
            )
            .unwrap();
    }

    let conn = db::open(&caminho).unwrap();

    // Dados antigos preservados, com o tipo padrão 'OS' e os itens ligados ao mesmo id.
    let (tipo, valor, pagamento): (String, f64, String) = conn
        .query_row("SELECT tipo, valor_total, pagamento FROM ordens_servico WHERE id = 1", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap();
    assert_eq!((tipo.as_str(), valor, pagamento.as_str()), ("OS", 30.0, "pix"));
    assert_eq!(conta(&conn, "SELECT COUNT(*) FROM ordem_itens WHERE os_id = 1"), 1);
    assert_eq!(conta(&conn, "SELECT COUNT(*) FROM ordens_servico"), 2);

    // Integridade referencial depois da reconstrução.
    assert_eq!(conta(&conn, "SELECT COUNT(*) FROM pragma_foreign_key_check"), 0);

    // O contador de ids não volta para o 3 (apagado): a próxima venda recebe o 4.
    abrir_caixa(&conn, 0.0);
    let nova = venda(&conn, json!({"funcionario_id": 1, "pagamento": "pix", "itens": [{"item_id": 1, "quantidade": 1}]})).unwrap();
    assert_eq!(nova["id"], 4);

    // A OS antiga continua listável com seu veículo e cliente.
    let os1 = ordens::buscar(&conn, 1).unwrap();
    assert_eq!(os1.veiculo_placa, "ABC1D23");
    assert_eq!(os1.cliente_nome, "Ana");
    assert_eq!(os1.tipo, "OS");

    drop(conn);
    let _ = std::fs::remove_file(&caminho);
}
