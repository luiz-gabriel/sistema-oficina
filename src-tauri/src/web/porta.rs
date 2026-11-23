//! Escolha da porta do servidor web: última porta salva, depois a 80, depois uma porta aleatória.
//! O socket escolhido já fica aberto (quem chama usa o próprio `TcpListener`), então não há corrida
//! entre "ver se a porta está livre" e "usar a porta". Nunca derruba quem já usa uma porta.
use std::fs;
use std::io;
use std::net::TcpListener;
use std::path::{Path, PathBuf};

pub const PORTA_PADRAO: u16 = 80;
const ARQUIVO: &str = "porta_web.txt";

/// De onde veio a porta escolhida.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origem {
    Salva,
    Padrao,
    Aleatoria,
}

impl Origem {
    pub fn nome(self) -> &'static str {
        match self {
            Origem::Salva => "salva",
            Origem::Padrao => "padrao",
            Origem::Aleatoria => "aleatoria",
        }
    }
}

pub struct Escolhida {
    pub listener: TcpListener,
    pub porta: u16,
    pub origem: Origem,
}

/// Tenta, em ordem: `salva` (se houver e for diferente de 0), 80 e, por fim, a porta 0 (o SO escolhe).
/// `ligar(p)` tenta abrir o socket na porta `p`; é injetado para os testes. Em produção é `ligar_lan`.
/// Erro só quando nem a porta aleatória abre.
pub fn escolher<F>(salva: Option<u16>, ligar: F) -> io::Result<Escolhida>
where
    F: Fn(u16) -> io::Result<TcpListener>,
{
    let salva = salva.filter(|p| *p != 0);
    if let Some(p) = salva {
        if let Ok(listener) = ligar(p) {
            return Ok(Escolhida { listener, porta: p, origem: Origem::Salva });
        }
    }
    // Se a salva já era a 80 (e falhou), não tenta de novo.
    if salva != Some(PORTA_PADRAO) {
        if let Ok(listener) = ligar(PORTA_PADRAO) {
            return Ok(Escolhida { listener, porta: PORTA_PADRAO, origem: Origem::Padrao });
        }
    }
    let listener = ligar(0)?;
    let porta = listener.local_addr()?.port();
    Ok(Escolhida { listener, porta, origem: Origem::Aleatoria })
}

/// Liga na interface da rede local (todas as interfaces do computador, sem roteador nem internet).
pub fn ligar_lan(porta: u16) -> io::Result<TcpListener> {
    TcpListener::bind(("0.0.0.0", porta))
}

/// Lê a porta salva no diretório de dados. Arquivo ausente ou ilegível vira None.
pub fn ler_salva(dir: &Path) -> Option<u16> {
    fs::read_to_string(caminho(dir)).ok()?.trim().parse().ok().filter(|p: &u16| *p != 0)
}

/// Salva a porta. Chamar somente depois de o bind ter dado certo.
pub fn gravar(dir: &Path, porta: u16) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    fs::write(caminho(dir), porta.to_string())
}

fn caminho(dir: &Path) -> PathBuf {
    dir.join(ARQUIVO)
}

/// Diz se o processo roda como root (Linux/macOS). Em root o servidor não sobe.
#[cfg(unix)]
pub fn rodando_como_root() -> bool {
    // A linha "Uid:" traz uid real, efetivo, salvo e do sistema de arquivos; basta o efetivo.
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("Uid:"))
                .and_then(|l| l.split_whitespace().nth(2).map(|v| v == "0"))
        })
        .unwrap_or(false)
}

#[cfg(not(unix))]
pub fn rodando_como_root() -> bool {
    false
}

#[cfg(test)]
mod testes {
    use super::*;
    use std::cell::RefCell;
    use std::net::TcpListener;

    fn livre() -> io::Result<TcpListener> {
        TcpListener::bind("127.0.0.1:0")
    }

    #[test]
    fn usa_80_quando_esta_livre_e_nao_ha_porta_salva() {
        let tentadas = RefCell::new(Vec::new());
        let e = escolher(None, |p| {
            tentadas.borrow_mut().push(p);
            if p == 80 { livre() } else { Err(io::Error::from(io::ErrorKind::PermissionDenied)) }
        })
        .expect("deve escolher");
        assert_eq!(e.porta, 80);
        assert_eq!(e.origem, Origem::Padrao);
        assert_eq!(*tentadas.borrow(), vec![80]);
    }

    #[test]
    fn com_80_ocupada_cai_para_porta_aleatoria() {
        let e = escolher(None, |p| {
            if p == 0 { livre() } else { Err(io::Error::from(io::ErrorKind::AddrInUse)) }
        })
        .expect("deve escolher");
        assert_eq!(e.origem, Origem::Aleatoria);
        assert_ne!(e.porta, 80);
        assert_eq!(e.porta, e.listener.local_addr().unwrap().port());
    }

    #[test]
    fn reutiliza_a_ultima_porta_salva_sem_tentar_80() {
        let tentadas = RefCell::new(Vec::new());
        let e = escolher(Some(5555), |p| {
            tentadas.borrow_mut().push(p);
            if p == 5555 { livre() } else { Err(io::Error::from(io::ErrorKind::AddrInUse)) }
        })
        .expect("deve escolher");
        assert_eq!(e.porta, 5555);
        assert_eq!(e.origem, Origem::Salva);
        assert_eq!(*tentadas.borrow(), vec![5555]);
    }

    #[test]
    fn salva_ocupada_cai_para_80_e_depois_aleatoria() {
        let tentadas = RefCell::new(Vec::new());
        let e = escolher(Some(5555), |p| {
            tentadas.borrow_mut().push(p);
            if p == 0 { livre() } else { Err(io::Error::from(io::ErrorKind::AddrInUse)) }
        })
        .expect("deve escolher");
        assert_eq!(e.origem, Origem::Aleatoria);
        assert_eq!(*tentadas.borrow(), vec![5555, 80, 0]);
    }

    #[test]
    fn sem_permissao_em_nenhuma_porta_fixa_devolve_aleatoria() {
        let tentadas = RefCell::new(Vec::new());
        let e = escolher(None, |p| {
            tentadas.borrow_mut().push(p);
            if p == 0 { livre() } else { Err(io::Error::from(io::ErrorKind::PermissionDenied)) }
        })
        .expect("deve escolher");
        assert_eq!(e.origem, Origem::Aleatoria);
        assert_eq!(*tentadas.borrow(), vec![80, 0]);
    }

    #[test]
    fn falha_total_devolve_erro() {
        let r = escolher(Some(5555), |_| Err(io::Error::from(io::ErrorKind::PermissionDenied)));
        assert!(r.is_err());
    }

    #[test]
    fn salva_80_nao_e_tentada_duas_vezes_quando_ocupada() {
        let tentadas = RefCell::new(Vec::new());
        let e = escolher(Some(80), |p| {
            tentadas.borrow_mut().push(p);
            if p == 0 { livre() } else { Err(io::Error::from(io::ErrorKind::AddrInUse)) }
        })
        .expect("deve escolher");
        assert_eq!(e.origem, Origem::Aleatoria);
        assert_eq!(*tentadas.borrow(), vec![80, 0]);
    }

    #[test]
    fn porta_salva_vai_e_volta_pelo_arquivo() {
        let dir = std::env::temp_dir().join(format!("oficina_porta_teste_{}", std::process::id()));
        assert_eq!(ler_salva(&dir), None);
        gravar(&dir, 4321).unwrap();
        assert_eq!(ler_salva(&dir), Some(4321));
        std::fs::write(caminho(&dir), "lixo").unwrap();
        assert_eq!(ler_salva(&dir), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
