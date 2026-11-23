//! Identidade TLS do servidor web: um certificado autoassinado por instalação, com chave própria
//! guardada na pasta de dados. A chave é reutilizada nas próximas inicializações; o certificado só é
//! recriado quando não existe. Navegadores ainda mostram aviso para certificado autoassinado.

use std::fs;
use std::io::Write;
use std::path::Path;

const ARQ_CERT: &str = "web_cert.pem";
const ARQ_CHAVE: &str = "web_chave.pem";

/// Certificado e chave privada em PEM.
pub struct Identidade {
    pub cert_pem: Vec<u8>,
    pub chave_pem: Vec<u8>,
}

/// Carrega a identidade da instalação ou cria uma nova, com os nomes e IPs informados no certificado.
pub fn identidade(dir: &Path, nomes: Vec<String>) -> Result<Identidade, String> {
    let cert_p = dir.join(ARQ_CERT);
    let chave_p = dir.join(ARQ_CHAVE);
    if let (Ok(cert), Ok(chave)) = (fs::read(&cert_p), fs::read(&chave_p)) {
        if !cert.is_empty() && !chave.is_empty() {
            return Ok(Identidade { cert_pem: cert, chave_pem: chave });
        }
    }
    let ck = rcgen::generate_simple_self_signed(nomes)
        .map_err(|e| format!("não consegui criar o certificado do servidor: {e}"))?;
    let cert_pem = ck.cert.pem().into_bytes();
    let chave_pem = ck.key_pair.serialize_pem().into_bytes();
    fs::create_dir_all(dir).map_err(|e| format!("não consegui criar a pasta de dados: {e}"))?;
    fs::write(&cert_p, &cert_pem).map_err(|e| format!("não consegui salvar o certificado: {e}"))?;
    escrever_privado(&chave_p, &chave_pem)?;
    Ok(Identidade { cert_pem, chave_pem })
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
    fn cria_identidade_e_reaproveita_a_mesma_chave() {
        let dir = pasta_teste("reuso");
        let primeira = identidade(&dir, vec!["localhost".into(), "127.0.0.1".into()]).unwrap();
        let segunda = identidade(&dir, vec!["outro.nome".into()]).unwrap();
        assert_eq!(primeira.chave_pem, segunda.chave_pem, "a chave deve ser preservada");
        assert_eq!(primeira.cert_pem, segunda.cert_pem, "o certificado existente é reutilizado");
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn chave_fica_com_permissao_restrita() {
        use std::os::unix::fs::PermissionsExt;
        let dir = pasta_teste("perm");
        identidade(&dir, vec!["localhost".into()]).unwrap();
        let modo = fs::metadata(dir.join(ARQ_CHAVE)).unwrap().permissions().mode() & 0o777;
        assert_eq!(modo, 0o600);
        let _ = fs::remove_dir_all(&dir);
    }
}
