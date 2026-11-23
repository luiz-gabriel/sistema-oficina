//! Hora atômica por SNTP (RFC 4330), cliente mínimo.
//! Usado só para registrar a data/hora da ativação. Se nenhum servidor responder, usa-se o relógio local.
//! A parte de montar/interpretar o pacote e a escolha da fonte são funções puras (testadas sem rede).
use std::net::{ToSocketAddrs, UdpSocket};
use std::time::Duration;

use chrono::{DateTime, Duration as Dur, Utc};
use serde_json::{json, Value};

/// Servidores tentados em ordem (UDP 123).
pub const SERVIDORES: [&str; 3] = ["pool.ntp.org:123", "time.google.com:123", "time.cloudflare.com:123"];
/// Tempo máximo de espera por servidor.
pub const TIMEOUT: Duration = Duration::from_secs(3);

pub const FONTE_NTP: &str = "ntp";
pub const FONTE_RELOGIO: &str = "relogio_local";

/// Segundos entre 1900-01-01 (época NTP) e 1970-01-01 (época Unix).
const DIFERENCA_EPOCA: i64 = 2_208_988_800;
const TAMANHO: usize = 48;
const MODO_CLIENTE: u8 = 3;
const MODO_SERVIDOR: u8 = 4;
const VERSAO: u8 = 4;

/// Resposta válida de um servidor.
#[derive(Debug, Clone, PartialEq)]
pub struct Consulta {
    pub servidor: String,
    /// Hora atômica no instante em que a resposta chegou.
    pub instante: DateTime<Utc>,
    /// Hora atômica menos relógio local, em segundos (positivo = relógio local está atrasado).
    pub desvio_segundos: f64,
}

/// Hora escolhida para a ativação, com a fonte usada.
#[derive(Debug, Clone, PartialEq)]
pub struct Hora {
    pub instante: DateTime<Utc>,
    /// `FONTE_NTP` ou `FONTE_RELOGIO`.
    pub fonte: &'static str,
    pub servidor: Option<String>,
    pub desvio_segundos: Option<f64>,
}

impl Hora {
    pub fn para_json(&self) -> Value {
        json!({
            "ativado_em": self.instante.to_rfc3339(),
            "fonte": self.fonte,
            "servidor": self.servidor,
            "desvio_segundos": self.desvio_segundos,
        })
    }
}

/// Monta o pedido SNTP (modo cliente). `transmissao` vai no campo de transmissão; o servidor
/// devolve esse valor no campo "originate", e assim confere-se que a resposta é deste pedido.
pub fn montar_pedido(transmissao: DateTime<Utc>) -> [u8; TAMANHO] {
    let mut p = [0u8; TAMANHO];
    // LI = 0 (sem aviso), versão 4, modo 3 (cliente).
    p[0] = (VERSAO << 3) | MODO_CLIENTE;
    p[40..48].copy_from_slice(&para_ntp(transmissao));
    p
}

/// Interpreta a resposta. Devolve o instante atômico (corrigido para `recebido_em`) e o desvio do relógio local.
/// Usa o cálculo de quatro marcas do RFC 4330: ((T2 - T1) + (T3 - T4)) / 2.
pub fn interpretar(
    resposta: &[u8],
    pedido: &[u8; TAMANHO],
    recebido_em: DateTime<Utc>,
) -> Result<(DateTime<Utc>, f64), String> {
    if resposta.len() < TAMANHO {
        return Err("resposta SNTP curta demais".into());
    }
    let li = resposta[0] >> 6;
    let modo = resposta[0] & 0x07;
    let estrato = resposta[1];
    if modo != MODO_SERVIDOR {
        return Err(format!("modo {modo} na resposta (esperado {MODO_SERVIDOR})"));
    }
    if li == 3 {
        return Err("servidor avisa que o relógio dele não está sincronizado".into());
    }
    if estrato == 0 || estrato > 15 {
        return Err(format!("estrato {estrato} inválido (recusa ou erro do servidor)"));
    }
    if resposta[24..32] != pedido[40..48] {
        return Err("resposta não corresponde a este pedido".into());
    }
    let t3_bytes = &resposta[40..48];
    if t3_bytes.iter().all(|&b| b == 0) {
        return Err("resposta sem hora de transmissão".into());
    }
    let t1 = de_ntp(&pedido[40..48]);
    let t2 = de_ntp(&resposta[32..40]);
    let t3 = de_ntp(t3_bytes);
    let t4 = recebido_em;
    let desvio = (segundos(t2 - t1) + segundos(t3 - t4)) / 2.0;
    let instante = t4 + Dur::microseconds((desvio * 1_000_000.0).round() as i64);
    Ok((instante, desvio))
}

/// Escolhe a fonte: a primeira tentativa bem-sucedida vence (as seguintes nem são avaliadas).
/// Sem nenhuma, usa o relógio local (chamado só nesse caso).
pub fn escolher<I, F>(tentativas: I, relogio_local: F) -> Hora
where
    I: IntoIterator<Item = Result<Consulta, String>>,
    F: FnOnce() -> DateTime<Utc>,
{
    match tentativas.into_iter().find_map(Result::ok) {
        Some(c) => Hora {
            instante: c.instante,
            fonte: FONTE_NTP,
            servidor: Some(c.servidor),
            desvio_segundos: Some(c.desvio_segundos),
        },
        None => Hora {
            instante: relogio_local(),
            fonte: FONTE_RELOGIO,
            servidor: None,
            desvio_segundos: None,
        },
    }
}

/// Consulta um servidor SNTP por UDP, com timeout. Não é chamada em testes automatizados.
pub fn consultar(servidor: &str, timeout: Duration) -> Result<Consulta, String> {
    let destino = servidor
        .to_socket_addrs()
        .map_err(|e| e.to_string())?
        .next()
        .ok_or_else(|| "endereço não resolvido".to_string())?;
    let ligacao = if destino.is_ipv4() { "0.0.0.0:0" } else { "[::]:0" };
    let sock = UdpSocket::bind(ligacao).map_err(|e| e.to_string())?;
    sock.set_read_timeout(Some(timeout)).map_err(|e| e.to_string())?;
    sock.set_write_timeout(Some(timeout)).map_err(|e| e.to_string())?;
    sock.connect(destino).map_err(|e| e.to_string())?;

    let pedido = montar_pedido(Utc::now());
    sock.send(&pedido).map_err(|e| e.to_string())?;
    let mut buf = [0u8; 128];
    let n = sock.recv(&mut buf).map_err(|e| e.to_string())?;
    let (instante, desvio_segundos) = interpretar(&buf[..n], &pedido, Utc::now())?;
    Ok(Consulta { servidor: servidor.to_string(), instante, desvio_segundos })
}

/// Obtém a hora na ordem dos servidores; sem resposta de nenhum, cai no relógio local.
pub fn obter_hora(servidores: &[&str], timeout: Duration) -> Hora {
    escolher(
        servidores.iter().map(|s| {
            consultar(s, timeout).map_err(|e| {
                eprintln!("aviso: SNTP {s} falhou: {e}");
                e
            })
        }),
        Utc::now,
    )
}

fn segundos(d: Dur) -> f64 {
    d.num_microseconds().unwrap_or(0) as f64 / 1_000_000.0
}

/// Instante Unix -> timestamp NTP de 64 bits (segundos + fração). Válido até 2036 (campo de 32 bits).
fn para_ntp(t: DateTime<Utc>) -> [u8; 8] {
    let segs = (t.timestamp() + DIFERENCA_EPOCA) as u32;
    let frac = ((t.timestamp_subsec_nanos() as u64) << 32) / 1_000_000_000;
    let mut out = [0u8; 8];
    out[..4].copy_from_slice(&segs.to_be_bytes());
    out[4..].copy_from_slice(&(frac as u32).to_be_bytes());
    out
}

fn de_ntp(b: &[u8]) -> DateTime<Utc> {
    let segs = u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as i64 - DIFERENCA_EPOCA;
    let frac = u32::from_be_bytes([b[4], b[5], b[6], b[7]]) as u64;
    let nanos = ((frac * 1_000_000_000) >> 32) as u32;
    DateTime::<Utc>::from_timestamp(segs, nanos).unwrap_or(DateTime::<Utc>::UNIX_EPOCH)
}

#[cfg(test)]
mod testes {
    use super::*;
    use chrono::TimeZone;

    fn base() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 22, 12, 0, 0).unwrap()
    }

    /// Resposta de servidor montada à mão (pacote de exemplo do teste).
    fn resposta_exemplo(pedido: &[u8; 48], t2: DateTime<Utc>, t3: DateTime<Utc>) -> [u8; 48] {
        let mut r = [0u8; 48];
        r[0] = (VERSAO << 3) | MODO_SERVIDOR; // LI 0, versão 4, modo 4 (servidor)
        r[1] = 2; // estrato 2
        r[2] = 6; // intervalo de consulta
        r[3] = 0xEC; // precisão
        r[24..32].copy_from_slice(&pedido[40..48]); // originate = nosso transmit
        r[32..40].copy_from_slice(&para_ntp(t2));
        r[40..48].copy_from_slice(&para_ntp(t3));
        r
    }

    #[test]
    fn pedido_tem_48_bytes_modo_cliente_versao_4() {
        let p = montar_pedido(base());
        assert_eq!(p.len(), 48);
        assert_eq!(p[0], 0x23);
        assert_eq!((p[0] >> 3) & 0x07, 4);
        assert_eq!(p[0] & 0x07, 3);
        assert_ne!(&p[40..48], &[0u8; 8]);
    }

    #[test]
    fn timestamp_ntp_ida_e_volta() {
        let t = Utc.with_ymd_and_hms(2026, 11, 22, 12, 0, 0).unwrap() + Dur::nanoseconds(123_456_789);
        let volta = de_ntp(&para_ntp(t));
        let diff = (volta - t).num_nanoseconds().unwrap().abs();
        assert!(diff < 1_000, "diferença de {diff} ns");
    }

    #[test]
    fn interpreta_resposta_com_relogio_local_atrasado_30s() {
        // Relógio local está 30 s atrás da hora atômica; atraso de ida e volta de 0,1 s cada.
        let t1 = base();
        let pedido = montar_pedido(t1);
        let t2 = t1 + Dur::milliseconds(30_100); // servidor recebe (hora atômica)
        let t3 = t2 + Dur::milliseconds(1); // servidor responde
        let resp = resposta_exemplo(&pedido, t2, t3);
        let t4 = t1 + Dur::milliseconds(200); // chegada, no relógio local
        let (instante, desvio) = interpretar(&resp, &pedido, t4).unwrap();
        assert!((desvio - 30.0).abs() < 0.01, "desvio {desvio}");
        let esperado = t4 + Dur::seconds(30);
        assert!((instante - esperado).num_milliseconds().abs() < 10);
    }

    #[test]
    fn recusa_respostas_invalidas() {
        let t1 = base();
        let pedido = montar_pedido(t1);
        let t2 = t1 + Dur::seconds(1);
        let bom = resposta_exemplo(&pedido, t2, t2);
        let t4 = t1 + Dur::seconds(2);
        assert!(interpretar(&bom, &pedido, t4).is_ok());

        // Curta demais.
        assert!(interpretar(&bom[..40], &pedido, t4).is_err());
        // Modo cliente (3) em vez de servidor (4).
        let mut r = bom;
        r[0] = (VERSAO << 3) | MODO_CLIENTE;
        assert!(interpretar(&r, &pedido, t4).is_err());
        // Estrato 0 (kiss-o'-death).
        let mut r = bom;
        r[1] = 0;
        assert!(interpretar(&r, &pedido, t4).is_err());
        // LI = 3 (relógio não sincronizado).
        let mut r = bom;
        r[0] |= 0xC0;
        assert!(interpretar(&r, &pedido, t4).is_err());
        // Originate diferente do pedido (resposta de outra consulta).
        let mut r = bom;
        r[24] ^= 0xFF;
        assert!(interpretar(&r, &pedido, t4).is_err());
        // Transmit zerado.
        let mut r = bom;
        r[40..48].fill(0);
        assert!(interpretar(&r, &pedido, t4).is_err());
    }

    fn ok(servidor: &str) -> Result<Consulta, String> {
        Ok(Consulta { servidor: servidor.into(), instante: base(), desvio_segundos: 1.5 })
    }

    #[test]
    fn escolhe_primeiro_servidor_que_responde_e_para_ali() {
        let mut chamadas = 0;
        let tentativas = (0..3).map(|i| {
            chamadas += 1;
            if i == 1 { ok("time.google.com") } else { Err("sem resposta".to_string()) }
        });
        let hora = escolher(tentativas, || panic!("não deve usar o relógio local"));
        assert_eq!(chamadas, 2);
        assert_eq!(hora.fonte, FONTE_NTP);
        assert_eq!(hora.servidor.as_deref(), Some("time.google.com"));
        assert_eq!(hora.desvio_segundos, Some(1.5));
        assert_eq!(hora.instante, base());
    }

    #[test]
    fn sem_resposta_usa_relogio_local() {
        let agora = Utc.with_ymd_and_hms(2026, 1, 2, 3, 4, 5).unwrap();
        let tentativas = vec![Err("timeout".to_string()), Err("timeout".to_string())];
        let hora = escolher(tentativas, || agora);
        assert_eq!(hora.fonte, FONTE_RELOGIO);
        assert_eq!(hora.instante, agora);
        assert_eq!(hora.servidor, None);
        assert_eq!(hora.desvio_segundos, None);
        let j = hora.para_json();
        assert_eq!(j["fonte"], "relogio_local");
        assert!(j["desvio_segundos"].is_null());
    }

    #[test]
    fn lista_de_servidores_tem_tres_padroes_na_porta_123() {
        assert_eq!(SERVIDORES.len(), 3);
        assert!(SERVIDORES.iter().all(|s| s.ends_with(":123")));
    }
}
