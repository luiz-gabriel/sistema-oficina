//! Validação de entrada: campos obrigatórios, placa e preço.
use oficina_core::clientes::{self, ClienteInput};
use oficina_core::db;
use oficina_core::error::AppError;
use oficina_core::validation::{is_placa_valida, normalize_placa};
use oficina_core::veiculos::{self, VeiculoInput};

#[test]
fn placa_aceita_formato_antigo_e_mercosul_normalizada() {
    assert_eq!(normalize_placa("abc-1d23"), "ABC1D23");
    assert!(is_placa_valida(&normalize_placa("abc-1d23")));
    assert!(is_placa_valida(&normalize_placa("ABC 1234")));
    assert!(!is_placa_valida("12345"));
}

#[test]
fn cliente_sem_nome_retorna_erro_de_campo_obrigatorio() {
    let conn = db::open_in_memory().unwrap();
    match clientes::criar(&conn, &ClienteInput::default()) {
        Err(AppError::Validation(erros)) => {
            assert!(erros.iter().any(|e| e.field == "nome" && e.message == "campo obrigatório"));
        }
        outro => panic!("esperava erro de validação, veio {outro:?}"),
    }
}

#[test]
fn veiculo_com_placa_invalida_ou_cliente_inexistente_e_recusado() {
    let conn = db::open_in_memory().unwrap();
    let placa_ruim = VeiculoInput { cliente_id: Some(1), placa: Some("123".into()), modelo: Some("Uno".into()), ..Default::default() };
    assert!(matches!(veiculos::criar(&conn, &placa_ruim), Err(AppError::Validation(_))));

    let sem_cliente = VeiculoInput { cliente_id: Some(99), placa: Some("ABC1234".into()), modelo: Some("Uno".into()), ..Default::default() };
    assert!(matches!(veiculos::criar(&conn, &sem_cliente), Err(AppError::NotFound(_))));
}
