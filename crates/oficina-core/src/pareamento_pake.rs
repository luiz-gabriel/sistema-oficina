//! Protocolo de pareamento do PC secundário com SPAKE2 (crate `spake2`, grupo Ed25519).
//!
//! O código de 6 dígitos é a senha do SPAKE2. Os dois lados trocam uma mensagem cada um e chegam à
//! mesma chave de sessão só se souberem o mesmo código; a conversa capturada não permite testar
//! códigos offline. Em cima da chave de sessão (HKDF):
//! - MAC de confirmação, nos dois sentidos (cliente prova que sabe o código; servidor prova que é o principal);
//! - chave de transporte (ChaCha20-Poly1305) para o token que sai na entrega;
//! - prova de consulta do pedido, para o aparelho consultar sem mandar segredo em claro.
//!
//! Este módulo não toca no banco: é só matemática e formato de mensagens (hex).
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Nonce};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use rand::RngCore;
use sha2::Sha256;
use spake2::{Ed25519Group, Identity, Password, Spake2};

const ID_PRINCIPAL: &[u8] = b"oficina/principal";
const ID_SECUNDARIO: &[u8] = b"oficina/secundario";
const INFO_CONFIRMACAO: &[u8] = b"oficina-pareamento/confirmacao/v1";
const INFO_TRANSPORTE: &[u8] = b"oficina-pareamento/transporte/v1";

/// Estado do PC secundário entre o envio da mensagem e a resposta do principal.
pub struct Iniciado(Spake2<Ed25519Group>);

/// Erros do protocolo. Mensagens genéricas: não dizem se o código estava certo ou errado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErroPake;

/// Passo 1 no PC secundário: a mensagem a enviar ao principal, a partir do código digitado.
pub fn cliente_iniciar(codigo: &str) -> (Iniciado, Vec<u8>) {
    let (estado, msg) = Spake2::<Ed25519Group>::start_a(
        &Password::new(codigo.as_bytes()),
        &Identity::new(ID_SECUNDARIO),
        &Identity::new(ID_PRINCIPAL),
    );
    (Iniciado(estado), msg)
}

/// Passo 1 no principal: responde à mensagem do aparelho e devolve a chave de sessão.
/// Com código errado a chave sai diferente, e a falha só aparece na confirmação (passo 2).
pub fn servidor_responder(codigo: &str, msg_cliente: &[u8]) -> Result<(Vec<u8>, Vec<u8>), ErroPake> {
    let (estado, msg_servidor) = Spake2::<Ed25519Group>::start_b(
        &Password::new(codigo.as_bytes()),
        &Identity::new(ID_SECUNDARIO),
        &Identity::new(ID_PRINCIPAL),
    );
    let chave = estado.finish(msg_cliente).map_err(|_| ErroPake)?;
    Ok((msg_servidor, chave))
}

/// Passo 1 no PC secundário: conclui com a mensagem do principal e devolve a chave de sessão.
pub fn cliente_concluir(iniciado: Iniciado, msg_servidor: &[u8]) -> Result<Vec<u8>, ErroPake> {
    iniciado.0.finish(msg_servidor).map_err(|_| ErroPake)
}

/// Chaves derivadas da chave de sessão, cada uma com um papel separado.
struct Chaves {
    confirmacao: [u8; 32],
    transporte: [u8; 32],
}

fn derivar(chave: &[u8]) -> Chaves {
    let hk = Hkdf::<Sha256>::new(None, chave);
    let mut confirmacao = [0u8; 32];
    let mut transporte = [0u8; 32];
    // 32 bytes sempre cabem no HKDF-SHA256 (limite de 8160 bytes).
    hk.expand(INFO_CONFIRMACAO, &mut confirmacao).expect("32 bytes cabem no HKDF");
    hk.expand(INFO_TRANSPORTE, &mut transporte).expect("32 bytes cabem no HKDF");
    Chaves { confirmacao, transporte }
}

/// HMAC com campos de tamanho prefixado, para a entrada não ser ambígua.
fn mac_de(chave: &[u8; 32], partes: &[&[u8]]) -> Hmac<Sha256> {
    let mut m = <Hmac<Sha256> as Mac>::new_from_slice(chave).expect("HMAC aceita qualquer tamanho de chave");
    for p in partes {
        m.update(&(p.len() as u32).to_be_bytes());
        m.update(p);
    }
    m
}

/// MAC de confirmação enviado pelo PC secundário (prova que ele conhece o código).
pub fn confirmar_cliente(chave: &[u8], msg_cliente: &[u8], msg_servidor: &[u8]) -> Vec<u8> {
    let c = derivar(chave);
    mac_de(&c.confirmacao, &[b"cliente", msg_cliente, msg_servidor]).finalize().into_bytes().to_vec()
}

/// MAC de confirmação devolvido pelo principal (prova que ele é quem gerou o código).
pub fn confirmar_servidor(chave: &[u8], msg_cliente: &[u8], msg_servidor: &[u8]) -> Vec<u8> {
    let c = derivar(chave);
    mac_de(&c.confirmacao, &[b"servidor", msg_cliente, msg_servidor]).finalize().into_bytes().to_vec()
}

/// Confere o MAC do principal em tempo constante (o aparelho só confia no principal que prova a chave).
pub fn conferir_servidor(chave: &[u8], msg_cliente: &[u8], msg_servidor: &[u8], recebido: &[u8]) -> bool {
    let c = derivar(chave);
    mac_de(&c.confirmacao, &[b"servidor", msg_cliente, msg_servidor]).verify_slice(recebido).is_ok()
}

/// Confere o MAC do PC secundário em tempo constante.
pub fn conferir_cliente(chave: &[u8], msg_cliente: &[u8], msg_servidor: &[u8], recebido: &[u8]) -> bool {
    let c = derivar(chave);
    mac_de(&c.confirmacao, &[b"cliente", msg_cliente, msg_servidor]).verify_slice(recebido).is_ok()
}

/// Prova de que o aparelho é o dono do pedido. Não é o segredo: vale só com a chave da sessão.
pub fn provar_consulta(chave: &[u8], pedido_id: i64) -> String {
    let c = derivar(chave);
    hex::encode(mac_de(&c.confirmacao, &[b"consulta", pedido_id.to_string().as_bytes()]).finalize().into_bytes())
}

/// Confere a prova de consulta em tempo constante.
pub fn conferir_consulta(chave: &[u8], pedido_id: i64, recebida_hex: &str) -> bool {
    let Ok(recebida) = hex::decode(recebida_hex) else { return false };
    let c = derivar(chave);
    mac_de(&c.confirmacao, &[b"consulta", pedido_id.to_string().as_bytes()]).verify_slice(&recebida).is_ok()
}

/// Cifra o token para a entrega. Saída em hex: nonce (12 bytes) seguido do texto cifrado.
pub fn cifrar_token(chave: &[u8], token: &str) -> String {
    let c = derivar(chave);
    let cifra = ChaCha20Poly1305::new((&c.transporte).into());
    let mut nonce = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce);
    // ChaCha20-Poly1305 não falha na cifra com entrada de tamanho razoável.
    let texto = cifra.encrypt(Nonce::from_slice(&nonce), token.as_bytes()).expect("cifra do token");
    let mut saida = nonce.to_vec();
    saida.extend_from_slice(&texto);
    hex::encode(saida)
}

/// Decifra o token entregue. Falha se a chave não for a da sessão ou se o texto foi alterado.
pub fn decifrar_token(chave: &[u8], cifrado_hex: &str) -> Result<String, ErroPake> {
    let bytes = hex::decode(cifrado_hex).map_err(|_| ErroPake)?;
    if bytes.len() < 12 {
        return Err(ErroPake);
    }
    let (nonce, texto) = bytes.split_at(12);
    let c = derivar(chave);
    let cifra = ChaCha20Poly1305::new((&c.transporte).into());
    let aberto = cifra.decrypt(Nonce::from_slice(nonce), texto).map_err(|_| ErroPake)?;
    String::from_utf8(aberto).map_err(|_| ErroPake)
}

#[cfg(test)]
mod testes {
    use super::*;

    /// Roda o protocolo inteiro em memória e devolve (chave do cliente, chave do servidor, msg_c, msg_s).
    fn rodar(codigo_cliente: &str, codigo_servidor: &str) -> (Result<Vec<u8>, ErroPake>, Result<Vec<u8>, ErroPake>, Vec<u8>, Vec<u8>) {
        let (ini, msg_c) = cliente_iniciar(codigo_cliente);
        let (msg_s, chave_s) = servidor_responder(codigo_servidor, &msg_c).unwrap();
        (cliente_concluir(ini, &msg_s), Ok(chave_s), msg_c, msg_s)
    }

    #[test]
    fn mesmo_codigo_gera_a_mesma_chave() {
        let (cliente, servidor, _, _) = rodar("123456", "123456");
        assert_eq!(cliente.unwrap(), servidor.unwrap());
    }

    #[test]
    fn codigo_errado_gera_chave_diferente_e_mac_falha() {
        let (cliente, servidor, msg_c, msg_s) = rodar("654321", "123456");
        let (ck, sk) = (cliente.unwrap(), servidor.unwrap());
        assert_ne!(ck, sk);
        let mac = confirmar_cliente(&ck, &msg_c, &msg_s);
        assert!(!conferir_cliente(&sk, &msg_c, &msg_s, &mac));
    }

    #[test]
    fn confirmacao_do_cliente_e_conferida_com_a_chave_certa() {
        let (cliente, servidor, msg_c, msg_s) = rodar("123456", "123456");
        let (ck, sk) = (cliente.unwrap(), servidor.unwrap());
        let mac = confirmar_cliente(&ck, &msg_c, &msg_s);
        assert!(conferir_cliente(&sk, &msg_c, &msg_s, &mac));
        // Mensagem trocada depois não confere (o MAC amarra a transcrição).
        assert!(!conferir_cliente(&sk, &msg_s, &msg_c, &mac));
    }

    #[test]
    fn mac_do_servidor_e_diferente_do_mac_do_cliente() {
        let (cliente, _, msg_c, msg_s) = rodar("123456", "123456");
        let ck = cliente.unwrap();
        assert_ne!(confirmar_cliente(&ck, &msg_c, &msg_s), confirmar_servidor(&ck, &msg_c, &msg_s));
    }

    #[test]
    fn token_so_abre_com_a_chave_da_sessao() {
        let (cliente, servidor, _, _) = rodar("123456", "123456");
        let (ck, sk) = (cliente.unwrap(), servidor.unwrap());
        let cifrado = cifrar_token(&sk, "token-secreto");
        assert_eq!(decifrar_token(&ck, &cifrado).unwrap(), "token-secreto");
        assert!(decifrar_token(&[7u8; 32], &cifrado).is_err());
        assert!(!cifrado.contains("token-secreto"));
    }

    #[test]
    fn token_adulterado_nao_abre() {
        let chave = [9u8; 32];
        let mut cifrado = cifrar_token(&chave, "abc");
        let ultimo = cifrado.pop().unwrap();
        cifrado.push(if ultimo == '0' { '1' } else { '0' });
        assert!(decifrar_token(&chave, &cifrado).is_err());
    }

    #[test]
    fn prova_de_consulta_depende_da_chave_e_do_pedido() {
        let chave = [5u8; 32];
        let prova = provar_consulta(&chave, 10);
        assert!(conferir_consulta(&chave, 10, &prova));
        assert!(!conferir_consulta(&chave, 11, &prova), "não serve para outro pedido");
        assert!(!conferir_consulta(&[6u8; 32], 10, &prova), "não serve com outra chave");
        assert!(!conferir_consulta(&chave, 10, "não é hex"));
    }
}
