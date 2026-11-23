// Carro com responsável precisa atravessar o snapshot de cópia para outro computador.
use oficina_core::clientes::{self, ClienteInput};
use oficina_core::funcionarios::{self, FuncionarioInput};
use oficina_core::{db, sync, veiculos::{self, VeiculoInput}};

#[test]
fn veiculo_com_responsavel_sobrevive_a_copia() {
    let mut a = db::open_in_memory().unwrap();
    let cli = clientes::criar(&a, &ClienteInput { nome: Some("Ana".into()), ..Default::default() }).unwrap().id;
    let func = funcionarios::criar(&a, &FuncionarioInput { nome: Some("Carlos".into()), ..Default::default() }).unwrap().id;
    veiculos::criar(&a, &VeiculoInput { cliente_id: Some(cli), placa: Some("ABC1D23".into()), modelo: Some("Gol".into()), responsavel_id: Some(func), ..Default::default() }).unwrap();
    let mut b = db::open_in_memory().unwrap();
    let snap = sync::snapshot(&a).unwrap();
    sync::aplicar_snapshot(&mut b, &snap).unwrap();
    let copiado = veiculos::listar(&b).unwrap();
    assert_eq!(copiado.len(), 1);
    assert_eq!(copiado[0].responsavel.as_deref(), Some("Carlos"));
    assert_eq!(sync::snapshot(&b).unwrap(), snap);
}

#[test]
fn snapshot_de_banco_antigo_migrado_copia_para_banco_novo() {
    let dir = std::env::temp_dir().join(format!("oficina-snap-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("antigo.db");
    let _ = std::fs::remove_file(&path);
    {
        let c = rusqlite::Connection::open(&path).unwrap();
        c.execute_batch("CREATE TABLE clientes (id INTEGER PRIMARY KEY, nome TEXT NOT NULL);
                         CREATE TABLE veiculos (id INTEGER PRIMARY KEY, cliente_id INTEGER, placa TEXT, marca TEXT, modelo TEXT, ano INTEGER);
                         INSERT INTO clientes (id, nome) VALUES (1, 'Ana');
                         INSERT INTO veiculos (id, cliente_id, placa, modelo) VALUES (1, 1, 'ABC1D23', 'Gol');").unwrap();
    }
    let a = db::open(&path).unwrap();
    let mut b = db::open_in_memory().unwrap();
    let snap = sync::snapshot(&a).unwrap();
    sync::aplicar_snapshot(&mut b, &snap).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}
