//! Rede local: conexão TCP com handshake que prova que a instalação é da mesma licença,
//! canal cifrado (ChaCha20-Poly1305) e descoberta de computadores na sub-rede.
//! O código de ativação nunca é enviado: só provas (HMAC) derivadas dele.
//! A identidade do usuário vem de um token emitido pelo servidor no login (ver `entrar`).
use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use oficina_core::rede_regras::{decidir_servidor, proxima_epoca, Decisao, Membro};
use oficina_core::seguranca::{chave_sessao, cifrar, decifrar, hmac_hex, nonce_aleatorio};
use oficina_core::sync::{CicloRede, SituacaoServidor};
use serde::Serialize;
use serde_json::{json, Value};

use crate::estado::Estado;

const DIR_CLIENTE: u8 = b'C';
const DIR_SERVIDOR: u8 = b'S';
const LIMITE_MENSAGEM: usize = 16 * 1024 * 1024;

/// Quem é esta instalação na rede.
#[derive(Clone)]
pub struct Identidade {
    pub chave: [u8; 32],
    pub dispositivo: String,
    pub computador: String,
    pub ativado_em: String,
    /// Época (termo) dos dados deste computador. Troca no ola e no desafio.
    pub epoca: u64,
}

/// O que um computador informa sobre si no handshake.
#[derive(Debug, Clone, Serialize)]
pub struct Info {
    pub dispositivo: String,
    pub computador: String,
    pub papel: String,
    pub ativado_em: String,
    /// Época dos dados do computador que respondeu.
    pub epoca: u64,
    pub endereco: String,
}

fn erro_io(m: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::Other, m.into())
}

fn enviar_claro(s: &mut TcpStream, v: &Value) -> io::Result<()> {
    let b = serde_json::to_vec(v).map_err(|e| erro_io(e.to_string()))?;
    s.write_all(&(b.len() as u32).to_be_bytes())?;
    s.write_all(&b)?;
    s.flush()
}

fn receber_claro(s: &mut TcpStream) -> io::Result<Value> {
    let mut len = [0u8; 4];
    s.read_exact(&mut len)?;
    let n = u32::from_be_bytes(len) as usize;
    if n > LIMITE_MENSAGEM {
        return Err(erro_io("mensagem grande demais"));
    }
    let mut b = vec![0u8; n];
    s.read_exact(&mut b)?;
    serde_json::from_slice(&b).map_err(|e| erro_io(e.to_string()))
}

fn hex_para_bytes(s: &str) -> io::Result<Vec<u8>> {
    hex::decode(s).map_err(|_| erro_io("dado inválido"))
}

/// Conexão já autenticada e cifrada.
pub struct Canal {
    stream: TcpStream,
    chave: [u8; 32],
    eu: u8,
    outro: u8,
    env: u64,
    rec: u64,
    pub info: Info,
}

impl Canal {
    pub fn enviar(&mut self, v: &Value) -> io::Result<()> {
        self.env += 1;
        let dados = serde_json::to_vec(v).map_err(|e| erro_io(e.to_string()))?;
        let c = cifrar(&self.chave, self.eu, self.env, &dados).map_err(|e| erro_io(e.to_string()))?;
        self.stream.write_all(&(c.len() as u32).to_be_bytes())?;
        self.stream.write_all(&c)?;
        self.stream.flush()
    }

    pub fn receber(&mut self) -> io::Result<Value> {
        let mut len = [0u8; 4];
        self.stream.read_exact(&mut len)?;
        let n = u32::from_be_bytes(len) as usize;
        if n > LIMITE_MENSAGEM {
            return Err(erro_io("mensagem grande demais"));
        }
        let mut c = vec![0u8; n];
        self.stream.read_exact(&mut c)?;
        self.rec += 1;
        let p = decifrar(&self.chave, self.outro, self.rec, &c).map_err(|e| erro_io(e.to_string()))?;
        serde_json::from_slice(&p).map_err(|e| erro_io(e.to_string()))
    }

    pub fn pedir(&mut self, v: &Value) -> io::Result<Value> {
        self.enviar(v)?;
        self.receber()
    }
}

fn configurar(s: &TcpStream, t: Duration) -> io::Result<()> {
    s.set_read_timeout(Some(t))?;
    s.set_write_timeout(Some(t))?;
    s.set_nodelay(true)
}

/// Cliente: conecta e prova que a licença é a mesma. Erro se o outro lado não tiver a mesma chave.
pub fn conectar(endereco: SocketAddr, id: &Identidade, espera: Duration) -> io::Result<Canal> {
    let mut s = TcpStream::connect_timeout(&endereco, Duration::from_millis(800))?;
    configurar(&s, espera)?;
    let nc = nonce_aleatorio();
    enviar_claro(&mut s, &json!({
        "t": "ola", "dispositivo": id.dispositivo, "computador": id.computador,
        "ativado_em": id.ativado_em, "epoca": id.epoca, "nc": hex::encode(nc)
    }))?;
    let d = receber_claro(&mut s)?;
    let ns = hex_para_bytes(d["ns"].as_str().unwrap_or(""))?;
    let esperada = hmac_hex(&id.chave, &[b"S", &nc, &ns]);
    if d["prova"].as_str() != Some(esperada.as_str()) {
        return Err(erro_io("o outro computador não é da mesma licença"));
    }
    let prova = hmac_hex(&id.chave, &[b"C", &nc, &ns]);
    enviar_claro(&mut s, &json!({"t": "prova", "prova": prova}))?;
    let info = Info {
        dispositivo: d["dispositivo"].as_str().unwrap_or("").into(),
        computador: d["computador"].as_str().unwrap_or("").into(),
        papel: d["papel"].as_str().unwrap_or("").into(),
        ativado_em: d["ativado_em"].as_str().unwrap_or("").into(),
        epoca: d["epoca"].as_u64().unwrap_or(0),
        endereco: endereco.to_string(),
    };
    Ok(Canal { stream: s, chave: chave_sessao(&id.chave, &nc, &ns), eu: DIR_CLIENTE, outro: DIR_SERVIDOR, env: 0, rec: 0, info })
}

/// Servidor: recebe a conexão, responde o desafio e confere a prova do cliente.
pub fn aceitar(mut s: TcpStream, id: &Identidade, papel: &str) -> io::Result<Canal> {
    configurar(&s, Duration::from_secs(10))?;
    let o = receber_claro(&mut s)?;
    let nc = hex_para_bytes(o["nc"].as_str().unwrap_or(""))?;
    let ns = nonce_aleatorio();
    let prova_srv = hmac_hex(&id.chave, &[b"S", &nc, &ns]);
    enviar_claro(&mut s, &json!({
        "t": "desafio", "dispositivo": id.dispositivo, "computador": id.computador, "papel": papel,
        "ativado_em": id.ativado_em, "epoca": id.epoca, "ns": hex::encode(ns), "prova": prova_srv
    }))?;
    let p = receber_claro(&mut s)?;
    if p["prova"].as_str() != Some(hmac_hex(&id.chave, &[b"C", &nc, &ns]).as_str()) {
        let _ = enviar_claro(&mut s, &json!({"t": "erro", "erro": "licença diferente"}));
        return Err(erro_io("cliente de outra licença"));
    }
    let info = Info {
        dispositivo: o["dispositivo"].as_str().unwrap_or("").into(),
        computador: o["computador"].as_str().unwrap_or("").into(),
        papel: "cliente".into(),
        ativado_em: o["ativado_em"].as_str().unwrap_or("").into(),
        epoca: o["epoca"].as_u64().unwrap_or(0),
        endereco: s.peer_addr().map(|a| a.to_string()).unwrap_or_default(),
    };
    Ok(Canal { stream: s, chave: chave_sessao(&id.chave, &nc, &ns), eu: DIR_SERVIDOR, outro: DIR_CLIENTE, env: 0, rec: 0, info })
}

/// Cliente: pede ao servidor a entrada do usuário. A senha vai cifrada no canal; a resposta traz o token.
/// Resposta: `{"ok": true, "token", "usuario", "nome", "perfil"}` ou `{"ok": false, "erro": {...}}`.
pub fn entrar(canal: &mut Canal, usuario: &str, senha: &str) -> io::Result<Value> {
    canal.pedir(&json!({"t": "login", "usuario": usuario, "senha": senha}))
}

/// Cliente: encerra a sessão no servidor (o token deixa de valer).
pub fn sair(canal: &mut Canal, token: &str) -> io::Result<()> {
    canal.enviar(&json!({"t": "logout", "token": token}))
}

/// IPv4 da interface principal (não envia pacote algum: só descobre a rota).
pub fn meu_ip() -> Option<Ipv4Addr> {
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("10.255.255.255:1").ok()?;
    match s.local_addr().ok()?.ip() {
        IpAddr::V4(v4) if !v4.is_loopback() => Some(v4),
        _ => None,
    }
}

/// Procura computadores da mesma licença na sub-rede /24, na porta indicada.
pub fn descobrir(portas: &[u16], id: &Identidade, extras: &[SocketAddr]) -> Vec<Info> {
    let mut alvos: Vec<SocketAddr> = extras.to_vec();
    if let Some(ip) = meu_ip() {
        let o = ip.octets();
        for &porta in portas {
            // Não se pula o próprio IP: outro computador pode estar nele (ex.: duas instâncias na mesma máquina).
            // Quem é este computador é reconhecido pelo identificador no handshake.
            for h in 1..=254u8 {
                alvos.push(SocketAddr::from(([o[0], o[1], o[2], h], porta)));
            }
        }
    }
    let (tx, rx) = mpsc::channel();
    for a in alvos {
        let tx = tx.clone();
        let id = id.clone();
        thread::spawn(move || {
            if let Ok(c) = conectar(a, &id, Duration::from_secs(3)) {
                let _ = tx.send(c.info.clone());
            }
        });
    }
    drop(tx);
    rx.iter().collect()
}

// ---- Computadores conhecidos da licença (base do quórum da eleição) ----

/// Guarda na configuração local os computadores da licença já vistos. Só cresce; não se apaga sozinho.
static REGISTRO: Mutex<()> = Mutex::new(());

fn conhecidos(est: &Estado) -> Vec<Value> {
    est.get("computadores_conhecidos", "")
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
}

pub fn lembrar(est: &Estado, dispositivo: &str, computador: &str, ativado_em: &str) {
    if dispositivo.is_empty() {
        return;
    }
    let _guarda = REGISTRO.lock().unwrap_or_else(|e| e.into_inner());
    let mut lista = conhecidos(est);
    match lista.iter_mut().find(|v| v["dispositivo"].as_str() == Some(dispositivo)) {
        Some(v) => {
            // Não troca uma data conhecida por uma vazia (peer de versão antiga).
            let data = if ativado_em.is_empty() { v["ativado_em"].as_str().unwrap_or("").to_string() } else { ativado_em.to_string() };
            *v = json!({"dispositivo": dispositivo, "computador": computador, "ativado_em": data});
        }
        None => lista.push(json!({"dispositivo": dispositivo, "computador": computador, "ativado_em": ativado_em})),
    }
    let _ = est.definir("computadores_conhecidos", &Value::Array(lista).to_string());
}

fn membro_de(info: &Info) -> Membro {
    Membro { dispositivo: info.dispositivo.clone(), ativado_em: info.ativado_em.clone(), papel: info.papel.clone(), epoca: info.epoca }
}

/// Este computador como membro da eleição.
fn eu_como_membro(id: &Identidade) -> Membro {
    Membro { dispositivo: id.dispositivo.clone(), ativado_em: id.ativado_em.clone(), papel: "cliente".into(), epoca: id.epoca }
}

/// Eleição: com os computadores que responderam agora, diz a situação do servidor (ver `sync::situacao_eleicao`).
/// Registra os computadores vistos antes de contar o quórum. Devolve também a época que ele teria ao promover.
pub fn avaliar_eleicao(est: &Estado, descobertos: &[Info]) -> (SituacaoServidor, u64) {
    let Ok(Some(id)) = est.identidade() else {
        return (SituacaoServidor::SemServidorSeguro { motivo: "computador sem identidade".into() }, 0);
    };
    for p in descobertos {
        lembrar(est, &p.dispositivo, &p.computador, &p.ativado_em);
    }
    lembrar(est, &id.dispositivo, &id.computador, &id.ativado_em);
    let total = conhecidos(est).len();
    let eu = eu_como_membro(&id);
    let outros: Vec<Membro> = descobertos.iter().map(membro_de).collect();
    (oficina_core::sync::situacao_eleicao(&eu, &outros, total), proxima_epoca(&eu, &outros))
}

/// Servidor: com os computadores que respondem agora, decide se continua servidor ou cede.
pub fn avaliar_servidor(est: &Estado, descobertos: &[Info]) -> Decisao {
    let Ok(Some(id)) = est.identidade() else {
        return Decisao::Manter;
    };
    for p in descobertos {
        lembrar(est, &p.dispositivo, &p.computador, &p.ativado_em);
    }
    let mut eu = eu_como_membro(&id);
    eu.papel = "servidor".into();
    let outros: Vec<Membro> = descobertos.iter().map(membro_de).collect();
    decidir_servidor(&eu, &outros)
}

static ESCUTA_ATIVA: AtomicBool = AtomicBool::new(false);
static ESCUTA_THREAD: Mutex<Option<JoinHandle<()>>> = Mutex::new(None);
/// Ciclo de rede (escuta + sincronização): o início e o fim passam por aqui, para que rede → local → rede funcione.
static CICLO: Mutex<CicloRede> = Mutex::new(CicloRede::novo());

fn ciclo_guardado() -> std::sync::MutexGuard<'static, CicloRede> {
    CICLO.lock().unwrap_or_else(|e| e.into_inner())
}

/// Inicia o ciclo de rede. Devolve a geração da execução, ou None se já está rodando.
pub fn ciclo_iniciar() -> Option<u64> {
    ciclo_guardado().iniciar()
}

/// Diz se a execução de geração `geracao` ainda é a vigente (falso depois de `parar_escuta`).
pub fn ciclo_vigente(geracao: u64) -> bool {
    ciclo_guardado().vigente(geracao)
}

/// Liga o servidor de rede. Cada conexão é tratada numa thread própria pelo `tratar`.
/// Não faz nada se a escuta já estiver ativa. Só deve ser chamada nos modos "servidor" e "cliente".
pub fn ouvir<F>(porta: u16, tratar: F) -> io::Result<()>
where
    F: Fn(TcpStream) + Send + Sync + 'static + Clone,
{
    if ESCUTA_ATIVA.swap(true, Ordering::SeqCst) {
        return Ok(());
    }
    let l = match TcpListener::bind(("0.0.0.0", porta)) {
        Ok(l) => l,
        Err(e) => {
            ESCUTA_ATIVA.store(false, Ordering::SeqCst);
            return Err(e);
        }
    };
    let h = thread::spawn(move || {
        for s in l.incoming() {
            if !ESCUTA_ATIVA.load(Ordering::SeqCst) {
                break;
            }
            if let Ok(s) = s {
                let t = tratar.clone();
                thread::spawn(move || t(s));
            }
        }
    });
    *ESCUTA_THREAD.lock().unwrap_or_else(|e| e.into_inner()) = Some(h);
    Ok(())
}

/// Para a escuta (ex.: o usuário passou a modo local). Acorda o servidor com uma conexão local
/// e espera a thread encerrar, para que uma nova escuta possa abrir a mesma porta.
pub fn parar_escuta(porta: u16) {
    // Para o ciclo mesmo que a escuta já estivesse fechada: a thread de sincronização sai na próxima volta.
    ciclo_guardado().parar();
    if !ESCUTA_ATIVA.swap(false, Ordering::SeqCst) {
        return;
    }
    let acordou = TcpStream::connect_timeout(&SocketAddr::from(([127, 0, 0, 1], porta)), Duration::from_millis(500)).is_ok();
    let h = ESCUTA_THREAD.lock().unwrap_or_else(|e| e.into_inner()).take();
    // Se não conseguiu acordar a thread, não espera (evita travar); ela sai na próxima conexão.
    if let (true, Some(h)) = (acordou, h) {
        let _ = h.join();
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn escuta_para_e_volta_a_subir_na_mesma_porta() {
        // Porta alta e fixa só para o teste; não usa dados nem a porta do aplicativo.
        let porta = 47391;
        ouvir(porta, |_s| {}).expect("primeira escuta");
        assert!(TcpStream::connect(("127.0.0.1", porta)).is_ok());
        parar_escuta(porta);
        assert!(TcpListener::bind(("0.0.0.0", porta)).is_ok(), "porta deve ficar livre após parar");
        ouvir(porta, |_s| {}).expect("religar a escuta");
        assert!(TcpStream::connect(("127.0.0.1", porta)).is_ok());
        parar_escuta(porta);
    }
}
