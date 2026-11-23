//! Identidade TLS do servidor web, com uma CA local por instalação.
//!
//! - A CA (`web_ca_cert.pem` / `web_ca_chave.pem`) é criada uma vez. O celular instala só o certificado
//!   público da CA (baixado em `/api/ca.crt`); a chave privada nunca sai do computador.
//! - O certificado do servidor é assinado pela CA e recriado quando a lista de nomes/IPs muda
//!   (por exemplo, quando o IP da rede local muda). A CA continua a mesma.
//! - Chaves privadas ficam com permissão 600 em Unix.

use std::fs;
use std::io::Write;
use std::path::Path;

use rcgen::{
    BasicConstraints, Certificate, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, KeyPair,
    KeyUsagePurpose,
};

const ARQ_CA_CERT: &str = "web_ca_cert.pem";
const ARQ_CA_CHAVE: &str = "web_ca_chave.pem";
const ARQ_CERT: &str = "web_cert.pem";
const ARQ_CHAVE: &str = "web_chave.pem";
const ARQ_NOMES: &str = "web_nomes.txt";

/// Certificado do servidor e a chave privada dele, em PEM.
pub struct Identidade {
    pub cert_pem: Vec<u8>,
    pub chave_pem: Vec<u8>,
}

/// Certificado público da CA local (o que o celular instala). `None` se a instalação ainda não tem CA.
pub fn ca_pem(dir: &Path) -> Option<Vec<u8>> {
    le(&dir.join(ARQ_CA_CERT))
}

/// Carrega a identidade da instalação ou cria o que faltar. `nomes` são os nomes e IPs do certificado.
pub fn identidade(dir: &Path, nomes: Vec<String>) -> Result<Identidade, String> {
    let mut nomes = nomes;
    nomes.sort();
    nomes.dedup();
    let lista = nomes.join("\n");
    let (_, ca, ca_chave) = carregar_ou_criar_ca(dir)?;

    let nomes_iguais = le(&dir.join(ARQ_NOMES)).is_some_and(|v| v == lista.as_bytes());
    if nomes_iguais {
        if let (Some(cert), Some(chave)) = (le(&dir.join(ARQ_CERT)), le(&dir.join(ARQ_CHAVE))) {
            return Ok(Identidade { cert_pem: cert, chave_pem: chave });
        }
    }

    let (cert, chave) = criar_certificado_servidor(nomes, &ca, &ca_chave)?;
    fs::create_dir_all(dir).map_err(|e| format!("não consegui criar a pasta de dados: {e}"))?;
    fs::write(dir.join(ARQ_CERT), &cert).map_err(|e| format!("não consegui salvar o certificado: {e}"))?;
    escrever_privado(&dir.join(ARQ_CHAVE), chave.as_bytes())?;
    fs::write(dir.join(ARQ_NOMES), lista.as_bytes()).map_err(|e| format!("não consegui salvar os nomes do certificado: {e}"))?;
    Ok(Identidade { cert_pem: cert.into_bytes(), chave_pem: chave.into_bytes() })
}

/// Lê a CA da instalação ou cria uma nova. Ao criar uma CA nova, o certificado do servidor antigo
/// deixa de valer (ele foi assinado por outra CA), então o registro de nomes é apagado.
fn carregar_ou_criar_ca(dir: &Path) -> Result<(Vec<u8>, Certificate, KeyPair), String> {
    if let (Some(cert), Some(chave)) = (le(&dir.join(ARQ_CA_CERT)), le(&dir.join(ARQ_CA_CHAVE))) {
        let chave_txt = String::from_utf8(chave).map_err(|e| format!("chave da CA inválida: {e}"))?;
        let cert_txt = String::from_utf8(cert.clone()).map_err(|e| format!("certificado da CA inválido: {e}"))?;
        let key = KeyPair::from_pem(&chave_txt).map_err(erro_rcgen)?;
        let ca = CertificateParams::from_ca_cert_pem(&cert_txt)
            .and_then(|p| p.self_signed(&key))
            .map_err(erro_rcgen)?;
        return Ok((cert, ca, key));
    }
    let mut p = CertificateParams::new(Vec::<String>::new()).map_err(erro_rcgen)?;
    p.distinguished_name.push(DnType::CommonName, "Oficina - CA local");
    p.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    p.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    let key = KeyPair::generate().map_err(erro_rcgen)?;
    let ca = p.self_signed(&key).map_err(erro_rcgen)?;
    let pem = ca.pem().into_bytes();
    fs::create_dir_all(dir).map_err(|e| format!("não consegui criar a pasta de dados: {e}"))?;
    fs::write(dir.join(ARQ_CA_CERT), &pem).map_err(|e| format!("não consegui salvar a CA: {e}"))?;
    escrever_privado(&dir.join(ARQ_CA_CHAVE), key.serialize_pem().as_bytes())?;
    let _ = fs::remove_file(dir.join(ARQ_NOMES));
    Ok((pem, ca, key))
}

/// Cria certificado de servidor com os nomes informados, assinado pela CA.
fn criar_certificado_servidor(nomes: Vec<String>, ca: &Certificate, ca_chave: &KeyPair) -> Result<(String, String), String> {
    let mut p = CertificateParams::new(nomes).map_err(erro_rcgen)?;
    p.distinguished_name.push(DnType::CommonName, "Oficina (servidor local)");
    p.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let chave = KeyPair::generate().map_err(erro_rcgen)?;
    let cert = p.signed_by(&chave, ca, ca_chave).map_err(erro_rcgen)?;
    Ok((cert.pem(), chave.serialize_pem()))
}

fn erro_rcgen(e: rcgen::Error) -> String {
    format!("não consegui criar o certificado do servidor: {e}")
}

fn le(p: &Path) -> Option<Vec<u8>> {
    fs::read(p).ok().filter(|v| !v.is_empty())
}

/// Grava a chave somente para o usuário do sistema (permissão 600 em Unix).
fn escrever_privado(p: &Path, dados: &[u8]) -> Result<(), String> {
    let erro = |e: std::io::Error| format!("não consegui salvar a chave do servidor: {e}");
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(p)
            .map_err(erro)?;
        f.write_all(dados).map_err(erro)?;
        fs::set_permissions(p, fs::Permissions::from_mode(0o600)).map_err(erro)?;
    }
    #[cfg(not(unix))]
    {
        let mut f = fs::File::create(p).map_err(erro)?;
        f.write_all(dados).map_err(erro)?;
    }
    Ok(())
}

#[cfg(test)]
mod testes {
    use super::*;

    fn pasta_teste(nome: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("oficina_tls_{nome}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        p
    }

    #[test]
    fn reaproveita_ca_e_chave_do_servidor() {
        let dir = pasta_teste("reuso");
        let primeira = identidade(&dir, vec!["localhost".into(), "127.0.0.1".into()]).unwrap();
        let segunda = identidade(&dir, vec!["localhost".into(), "127.0.0.1".into()]).unwrap();
        let ca_antes = ca_pem(&dir);
        let _ = identidade(&dir, vec!["localhost".into()]).unwrap();
        assert_eq!(ca_antes, ca_pem(&dir), "a CA deve ser preservada");
        assert_eq!(primeira.chave_pem, segunda.chave_pem, "a chave do servidor deve ser preservada");
        assert_eq!(primeira.cert_pem, segunda.cert_pem, "o certificado existente é reutilizado");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn muda_certificado_quando_o_ip_muda_mas_mantem_a_ca() {
        let dir = pasta_teste("ip");
        let antes = identidade(&dir, vec!["localhost".into(), "192.168.0.10".into()]).unwrap();
        let depois = identidade(&dir, vec!["localhost".into(), "192.168.0.20".into()]).unwrap();
        assert!(ca_pem(&dir).is_some());
        assert_ne!(antes.cert_pem, depois.cert_pem, "o certificado do servidor deve ser refeito");
                let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn ca_nova_invalida_o_certificado_antigo() {
        let dir = pasta_teste("ca_nova");
        let antes = identidade(&dir, vec!["localhost".into()]).unwrap();
        let ca_antes = ca_pem(&dir);
        fs::remove_file(dir.join(ARQ_CA_CERT)).unwrap();
        fs::remove_file(dir.join(ARQ_CA_CHAVE)).unwrap();
        let depois = identidade(&dir, vec!["localhost".into()]).unwrap();
        assert_ne!(ca_antes, ca_pem(&dir));
        assert_ne!(antes.cert_pem, depois.cert_pem, "certificado antigo não pode sobreviver a uma CA nova");
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn chaves_ficam_com_permissao_restrita() {
        use std::os::unix::fs::PermissionsExt;
        let dir = pasta_teste("perm");
        identidade(&dir, vec!["localhost".into()]).unwrap();
        for arq in [ARQ_CHAVE, ARQ_CA_CHAVE] {
            let modo = fs::metadata(dir.join(arq)).unwrap().permissions().mode() & 0o777;
            assert_eq!(modo, 0o600, "{arq}");
        }
        let _ = fs::remove_dir_all(&dir);
    }
}
