// Motor do veículo: opcional, gravado e devolvido; banco antigo (sem coluna motor) é migrado ao abrir.
use oficina_core::clientes::{self, ClienteInput};
use oficina_core::{db, veiculos::{self, VeiculoInput}};

fn cliente(conn: &rusqlite::Connection) -> i64 {
    clientes::criar(conn, &ClienteInput { nome: Some("Ana".into()), ..Default::default() }).unwrap().id
}

#[test]
fn motor_e_gravado_e_opcional() {
    let conn = db::open_in_memory().unwrap();
    let cli = cliente(&conn);
    let com = veiculos::criar(&conn, &VeiculoInput {
        cliente_id: Some(cli), placa: Some("ABC1D23".into()), modelo: Some("Gol".into()),
        motor: Some("1.6".into()), ..Default::default()
    }).unwrap();
    assert_eq!(com.motor.as_deref(), Some("1.6"));
    let sem = veiculos::criar(&conn, &VeiculoInput {
        cliente_id: Some(cli), placa: Some("DEF2G34".into()), modelo: Some("Uno".into()), ..Default::default()
    }).unwrap();
    assert_eq!(sem.motor, None);
}

#[test]
fn banco_antigo_ganha_coluna_motor_ao_abrir() {
    let dir = std::env::temp_dir().join(format!("oficina-mig-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("antigo.db");
    let _ = std::fs::remove_file(&path);
    {
        // esquema da versão anterior: veiculos sem a coluna motor
        let c = rusqlite::Connection::open(&path).unwrap();
        c.execute_batch("CREATE TABLE veiculos (id INTEGER PRIMARY KEY, cliente_id INTEGER, placa TEXT, marca TEXT, modelo TEXT, ano INTEGER);").unwrap();
    }
    let conn = db::open(&path).unwrap();
    let colunas: Vec<String> = conn.prepare("PRAGMA table_info(veiculos)").unwrap()
        .query_map([], |r| r.get::<_, String>(1)).unwrap().collect::<Result<_, _>>().unwrap();
    assert!(colunas.iter().any(|c| c == "motor"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn banco_antigo_de_funcionarios_ganha_login_e_perfil() {
    let dir = std::env::temp_dir().join(format!("oficina-mig-func-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("antigo.db");
    let _ = std::fs::remove_file(&path);
    {
        // esquema das versões anteriores: funcionarios sem login/senha/perfil
        let c = rusqlite::Connection::open(&path).unwrap();
        c.execute_batch("CREATE TABLE funcionarios (id INTEGER PRIMARY KEY AUTOINCREMENT, nome TEXT NOT NULL, cargo TEXT, telefone TEXT);
                         INSERT INTO funcionarios (nome) VALUES ('Antigo');").unwrap();
    }
    let conn = db::open(&path).expect("abre banco antigo sem erro");
    let lista = oficina_core::funcionarios::listar(&conn).unwrap();
    assert_eq!(lista.len(), 1);
    assert!(!lista[0].tem_acesso);
    assert_eq!(lista[0].perfil, "funcionario");
    let _ = std::fs::remove_dir_all(&dir);
}
