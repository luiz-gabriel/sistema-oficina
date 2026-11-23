//! Estado compartilhado do aplicativo: banco, sessão do usuário e situação da rede.
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::ops::{Deref, DerefMut};
use std::sync::{Mutex, RwLock, RwLockReadGuard};
use std::time::{Duration, Instant};

use oficina_core::comandos::Contexto;
use oficina_core::error::{AppError, AppResult};
use oficina_core::db::{ConexaoPiscina, Piscina};
use oficina_core::{ativacao, config, db};
use rusqlite::Connection;

use crate::rede::Identidade;

pub struct Rede {
    /// Último contato bem-sucedido com o servidor (modo cliente).
    pub conectado: AtomicBool,
    pub falhas: AtomicU32,
    /// Quando cada computador cliente falou com este servidor pela última vez.
    pub vistos: Mutex<HashMap<String, Instant>>,
    /// Sessões de login emitidas por este computador quando atua como servidor.
    pub sessoes: oficina_core::rede_regras::Sessoes,
    /// Token de sessão obtido no servidor, quando este computador atua como cliente.
    pub token: Mutex<Option<String>>,
    pub houve_clientes: AtomicBool,
    pub ultima_busca: Mutex<Option<Instant>>,
}

/// Conexão emprestada do banco. Enquanto existir, o banco não pode ser trocado (restauração espera).
pub struct Conexao<'a> {
    // Ordem importa: a conexão volta à piscina antes de soltar a trava.
    conn: ConexaoPiscina,
    _trava: RwLockReadGuard<'a, Option<Piscina>>,
}

impl Deref for Conexao<'_> {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        &self.conn
    }
}

impl DerefMut for Conexao<'_> {
    fn deref_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }
}

pub struct Estado {
    /// Piscina de conexões ao banco. `None` só durante a restauração de um backup.
    pub piscina: RwLock<Option<Piscina>>,
    pub sessao: Mutex<Option<Contexto>>,
    pub rede: Rede,
    pub dir: PathBuf,
    pub porta: u16,
    pub computador: String,
    /// Servidor web local (navegador da rede local). Ver `web`.
    pub web: crate::web::Web,
}

impl Estado {
    /// Abre o banco da instalação (cria o esquema se preciso) e a piscina de conexões.
    pub fn novo(dir: PathBuf, porta: u16) -> AppResult<Estado> {
        let caminho = dir.join("oficina.db");
        db::open(&caminho)?;
        let piscina = db::piscina(&caminho)?;
        Ok(Estado::com_piscina(piscina, dir, porta))
    }

    fn com_piscina(piscina: Piscina, dir: PathBuf, porta: u16) -> Estado {
        let computador = std::fs::read_to_string("/etc/hostname")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "Computador".into());
        Estado {
            piscina: RwLock::new(Some(piscina)),
            sessao: Mutex::new(None),
            rede: Rede {
                conectado: AtomicBool::new(false),
                falhas: AtomicU32::new(0),
                vistos: Mutex::new(HashMap::new()),
                sessoes: Default::default(),
                token: Mutex::new(None),
                houve_clientes: AtomicBool::new(false),
                ultima_busca: Mutex::new(None),
            },
            dir,
            porta,
            computador,
            web: Default::default(),
        }
    }

    /// Pega uma conexão da piscina. Várias requisições leem ao mesmo tempo; a escrita espera o bloqueio do SQLite.
    pub fn db(&self) -> AppResult<Conexao<'_>> {
        let trava = self.piscina.read().map_err(|_| AppError::Database("conexão indisponível".into()))?;
        let conn = match trava.as_ref() {
            Some(p) => p.get().map_err(|e| AppError::Database(format!("sem conexão disponível com o banco: {e}")))?,
            None => return Err(AppError::Database("o banco está sendo trocado, tente de novo.".into())),
        };
        Ok(Conexao { conn, _trava: trava })
    }

    pub fn get(&self, chave: &str, padrao: &str) -> AppResult<String> {
        let c = self.db()?;
        config::obter_ou(&c, chave, padrao)
    }

    pub fn definir(&self, chave: &str, valor: &str) -> AppResult<()> {
        let c = self.db()?;
        config::definir(&c, chave, valor)
    }

    /// "local", "servidor", "cliente" ou "" (ainda não escolhido).
    pub fn modo(&self) -> String {
        self.get("modo", "").unwrap_or_default()
    }

    /// Época (termo) dos dados deste computador. Sobe a cada promoção a servidor; cópias trazem a do servidor.
    pub fn epoca(&self) -> u64 {
        self.get("epoca", "0").ok().and_then(|v| v.parse().ok()).unwrap_or(0)
    }

    pub fn offline(&self) -> bool {
        self.get("offline", "0").map(|v| v == "1").unwrap_or(false)
    }

    pub fn servidor_addr(&self) -> Option<String> {
        self.get("servidor_addr", "").ok().filter(|s| !s.is_empty())
    }

    pub fn identidade(&self) -> AppResult<Option<Identidade>> {
        let c = self.db()?;
        if !ativacao::esta_ativado(&c)? {
            return Ok(None);
        }
        let chave_hex = config::obter_ou(&c, "chave_rede", "")?;
        let bytes = hex::decode(chave_hex).map_err(|_| AppError::Database("chave da rede inválida".into()))?;
        let chave: [u8; 32] = bytes.try_into().map_err(|_| AppError::Database("chave da rede inválida".into()))?;
        Ok(Some(Identidade {
            chave,
            dispositivo: config::obter_ou(&c, "dispositivo_id", "")?,
            computador: self.computador.clone(),
            ativado_em: config::obter_ou(&c, "ativado_em", "")?,
            // Lido pelo mesmo `c`: `self.epoca()` pegaria o lock do banco de novo (travaria).
            epoca: config::obter_ou(&c, "epoca", "0")?.parse().unwrap_or(0),
        }))
    }

    /// Papel deste computador agora: servidor, cliente, sem_servidor, isolado, local ou indefinido.
    pub fn papel(&self) -> String {
        match self.modo().as_str() {
            "servidor" => if self.offline() { "isolado" } else { "servidor" }.into(),
            "cliente" => if self.offline() || self.rede.conectado.load(Ordering::Relaxed) { "cliente" } else { "sem_servidor" }.into(),
            "local" => "local".into(),
            _ => "indefinido".into(),
        }
    }

    pub fn ja_visto(&self, dispositivo: &str) {
        if let Ok(mut m) = self.rede.vistos.lock() {
            m.insert(dispositivo.to_string(), Instant::now());
            self.rede.houve_clientes.store(true, Ordering::Relaxed);
        }
    }

    /// Clientes que falaram com o servidor nos últimos `janela`.
    pub fn clientes_recentes(&self, janela: Duration) -> usize {
        self.rede.vistos.lock().map(|m| m.values().filter(|t| t.elapsed() < janela).count()).unwrap_or(0)
    }

    /// Portas onde procurar outros computadores. Padrão: a porta deste computador.
    /// `OFICINA_DESCOBERTA=7878,7879` amplia a busca (usado para testar vários computadores numa máquina só).
    pub fn portas_descoberta(&self) -> Vec<u16> {
        let mut portas: Vec<u16> = std::env::var("OFICINA_DESCOBERTA")
            .ok()
            .map(|v| v.split(',').filter_map(|p| p.trim().parse().ok()).collect())
            .unwrap_or_default();
        if !portas.contains(&self.porta) {
            portas.push(self.porta);
        }
        portas
    }

    pub fn endereco_servidor(&self) -> AppResult<SocketAddr> {
        let s = self.servidor_addr().ok_or_else(|| AppError::Offline("Nenhum servidor configurado.".into()))?;
        s.parse().map_err(|_| AppError::Offline(format!("Endereço do servidor inválido: {s}")))
    }
}

pub fn contexto_sistema() -> Contexto {
    Contexto { usuario: "sistema".into(), perfil: "administrador".into(), computador: String::new() }
}
