// Responsável pelo carro: opcional, precisa existir, aparece com o nome, e banco antigo ganha a coluna.
use oficina_core::clientes::{self, ClienteInput};
use oficina_core::funcionarios::{self, FuncionarioInput};
use oficina_core::{db, error::AppError, veiculos::{self, VeiculoInput}};

fn base(conn: &rusqlite::Connection) -> (i64, i64) {
    let cli = clientes::criar(conn, &ClienteInput { nome: Some("Ana".into()), ..Default::default() }).unwrap().id;
    let func = funcionarios::criar(conn, &FuncionarioInput { nome: Some("Carlos".into()), ..Default::default() }).unwrap().id;
    (cli, func)
}

#[test]
fn responsavel_e_opcional_e_devolve_o_nome() {
    let conn = db::open_in_memory().unwrap();
    let (cli, func) = base(&conn);
    let com = veiculos::criar(&conn, &VeiculoInput {
        cliente_id: Some(cli), placa: Some("ABC1D23".into()), modelo: Some("Gol".into()),
        responsavel_id: Some(func), ..Default::default()
    }).unwrap();
    assert_eq!(com.responsavel_id, Some(func));
    assert_eq!(com.responsavel.as_deref(), Some("Carlos"));
    let sem = veiculos::criar(&conn, &VeiculoInput {
        cliente_id: Some(cli), placa: Some("DEF2G34".into()), modelo: Some("Uno".into()), ..Default::default()
    }).unwrap();
    assert_eq!(sem.responsavel_id, None);
    assert_eq!(sem.responsavel, None);
}

#[test]
fn responsavel_inexistente_e_recusado_com_campo_claro() {
    let conn = db::open_in_memory().unwrap();
    let (cli, _) = base(&conn);
    let err = veiculos::criar(&conn, &VeiculoInput {
        cliente_id: Some(cli), placa: Some("ABC1D23".into()), modelo: Some("Gol".into()),
        responsavel_id: Some(999), ..Default::default()
    }).unwrap_err();
    match err {
        AppError::Validation(erros) => assert!(erros.iter().any(|e| e.field == "responsavel_id")),
        outro => panic!("esperava erro de validação, veio {outro:?}"),
    }
}

#[test]
fn banco_antigo_ganha_coluna_responsavel() {
    let dir = std::env::temp_dir().join(format!("oficina-resp-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("antigo.db");
    let _ = std::fs::remove_file(&path);
    {
        let c = rusqlite::Connection::open(&path).unwrap();
        c.execute_batch(
            "CREATE TABLE clientes (id INTEGER PRIMARY KEY AUTOINCREMENT, nome TEXT NOT NULL, telefone TEXT, email TEXT, endereco TEXT, observacoes TEXT);
             CREATE TABLE veiculos (id INTEGER PRIMARY KEY AUTOINCREMENT, cliente_id INTEGER NOT NULL REFERENCES clientes(id), placa TEXT NOT NULL UNIQUE, marca TEXT, modelo TEXT NOT NULL, ano INTEGER, motor TEXT, cor TEXT);
             INSERT INTO clientes (nome) VALUES ('Antigo');
             INSERT INTO veiculos (cliente_id, placa, modelo) VALUES (1, 'ABC1D23', 'Gol');"
        ).unwrap();
    }
    let conn = db::open(&path).unwrap();
    let colunas: Vec<String> = conn.prepare("PRAGMA table_info(veiculos)").unwrap()
        .query_map([], |r| r.get::<_, String>(1)).unwrap().collect::<Result<_, _>>().unwrap();
    assert!(colunas.iter().any(|c| c == "responsavel_id"));
    assert_eq!(veiculos::listar(&conn).unwrap().len(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}
