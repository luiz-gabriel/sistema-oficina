// Leitor de código de barras (busca exata, código único) e envio do cupom à impressora térmica.
use std::io::Read;
use std::net::TcpListener;
use std::thread;

use oficina_core::itens::{self, ItemInput};
use oficina_core::{db, error::AppError, impressao};

fn peca(nome: &str, codigo: Option<&str>, venda: f64) -> ItemInput {
    ItemInput {
        nome: Some(nome.into()),
        tipo: Some("peca".into()),
        codigo: codigo.map(String::from),
        preco_venda: Some(venda),
        quantidade: Some(10),
        ..Default::default()
    }
}

#[test]
fn busca_pelo_codigo_exato() {
    let conn = db::open_in_memory().unwrap();
    let oleo = itens::criar(&conn, &peca("Óleo 5W30", Some("7891234567895"), 50.0)).unwrap();
    itens::criar(&conn, &peca("Filtro", Some("7890000000012"), 30.0)).unwrap();
    // Leitor manda o código com espaços às vezes: a busca ignora as bordas.
    assert_eq!(itens::buscar_por_codigo(&conn, "  7891234567895 ").unwrap().id, oleo.id);
    assert!(matches!(itens::buscar_por_codigo(&conn, "789123"), Err(AppError::NotFound(_))));
    assert!(matches!(itens::buscar_por_codigo(&conn, ""), Err(AppError::NotFound(_))));
}

#[test]
fn dois_itens_nao_podem_ter_o_mesmo_codigo() {
    let conn = db::open_in_memory().unwrap();
    let a = itens::criar(&conn, &peca("A", Some("111"), 1.0)).unwrap();
    assert!(matches!(itens::criar(&conn, &peca("B", Some("111"), 1.0)), Err(AppError::Validation(_))));
    // Atualizar o próprio item com o mesmo código continua valendo.
    itens::atualizar(&conn, a.id, &peca("A2", Some("111"), 2.0)).unwrap();
    // Sem código, vários itens podem existir.
    itens::criar(&conn, &peca("C", None, 1.0)).unwrap();
    itens::criar(&conn, &peca("D", None, 1.0)).unwrap();
}

#[test]
fn destino_da_impressora_e_cupom() {
    assert!(impressao::destino("192.168.0.50:9100").is_ok());
    assert!(impressao::destino("192.168.0.50").is_err());
}

#[test]
fn envia_o_cupom_pela_rede() {
    // Um servidor local faz o papel da impressora térmica de rede (porta 9100).
    let ouvinte = TcpListener::bind("127.0.0.1:0").unwrap();
    let porta = ouvinte.local_addr().unwrap().port();
    let recebido = thread::spawn(move || {
        let (mut con, _) = ouvinte.accept().unwrap();
        let mut dados = Vec::new();
        con.read_to_end(&mut dados).unwrap();
        dados
    });
    let dest = impressao::destino(&format!("127.0.0.1:{porta}")).unwrap();
    impressao::enviar(&dest, &impressao::teste().unwrap()).unwrap();
    let dados = recebido.join().unwrap();
    assert_eq!(&dados[..2], &[0x1B, 0x40], "começa reiniciando a impressora");
    let texto = String::from_utf8_lossy(&dados);
    assert!(texto.contains("TESTE DE IMPRESSAO"));
    assert_eq!(&dados[dados.len() - 4..dados.len() - 2], &[0x1D, 0x56], "termina cortando o papel");
}

#[test]
fn impressora_inexistente_mostra_erro_claro() {
    // Porta fechada: nada escuta aqui.
    let dest = impressao::destino("127.0.0.1:1").unwrap();
    match impressao::enviar(&dest, &impressao::teste().unwrap()) {
        Err(AppError::Conflict(msg)) => assert!(msg.contains("127.0.0.1:1"), "{msg}"),
        outro => panic!("esperava erro de conexão, veio {outro:?}"),
    }
}
