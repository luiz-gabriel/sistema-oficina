//! Regras de rede sem socket nem Tauri (testáveis isoladamente):
//! quais modos abrem rede, eleição de servidor e sessões de login emitidas pelo servidor.
use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use chrono::{DateTime, FixedOffset};

use crate::seguranca::nonce_aleatorio;

/// Só "servidor" e "cliente" abrem escuta e conexões. "local", vazio ou "indefinido" não abrem nada.
pub fn modo_usa_rede(modo: &str) -> bool {
    matches!(modo, "servidor" | "cliente")
}

/// Um computador da licença, do ponto de vista da eleição.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Membro {
    pub dispositivo: String,
    /// Data de ativação gravada na instalação (RFC 3339).
    pub ativado_em: String,
    /// "servidor", "cliente", "sem_servidor", "isolado", ...
    pub papel: String,
    /// Época (termo) dos dados que este computador tem. Sobe a cada promoção a servidor.
    pub epoca: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decisao {
    /// Este computador vira servidor.
    Promover,
    /// Este computador era servidor e passa a cliente: há dados mais recentes ou outro servidor tem prioridade.
    Ceder,
    /// Este computador continua servidor.
    Manter,
    /// Nenhuma mudança agora (o motivo é só para leitura e registro).
    Aguardar(&'static str),
}

/// Data de ativação como instante (fuso considerado). Ausente ou inválida: None, fica por último.
fn antiguidade(ativado_em: &str) -> Option<DateTime<FixedOffset>> {
    DateTime::parse_from_rfc3339(ativado_em).ok()
}

/// Computadores distintos dos `outros` (pelo identificador do dispositivo, sem contar `eu`). Respostas repetidas contam uma vez.
fn unicos<'a>(eu: &Membro, outros: &'a [Membro]) -> Vec<&'a Membro> {
    let mapa: BTreeMap<&str, &Membro> = outros
        .iter()
        .filter(|m| m.dispositivo != eu.dispositivo)
        .map(|m| (m.dispositivo.as_str(), m))
        .collect();
    mapa.into_values().collect()
}

/// Precedência de um computador (menor vence): época mais alta; depois ativação mais antiga
/// (data ausente ou inválida fica por último); depois o identificador do dispositivo.
/// Não depende da ordem em que os computadores foram descobertos.
type Precedencia<'a> = (std::cmp::Reverse<u64>, bool, Option<DateTime<FixedOffset>>, &'a str);

fn precedencia(m: &Membro) -> Precedencia<'_> {
    let ativacao = antiguidade(&m.ativado_em);
    (std::cmp::Reverse(m.epoca), ativacao.is_none(), ativacao, m.dispositivo.as_str())
}

/// Eleição de servidor quando não há servidor (função pura).
///
/// 1. Se outro computador responde como servidor, ninguém se promove.
/// 2. Quórum: os computadores que respondem (incluindo este) precisam ser maioria estrita
///    dos conhecidos. Duas partes isoladas não podem ter maioria juntas, então no máximo uma se promove.
///    Consequência conhecida: com 2 de 3 computadores fora do ar, o sobrevivente não se promove sozinho.
/// 3. Entre os candidatos (este e os "cliente"/"sem_servidor" que respondem), vence a precedência:
///    época mais alta (dados mais recentes), depois ativação mais antiga, depois identificador.
///    "isolado" não é candidato.
pub fn decidir_promocao(eu: &Membro, outros: &[Membro], total_conhecidos: usize) -> Decisao {
    let outros = unicos(eu, outros);
    if outros.iter().any(|m| m.papel == "servidor") {
        return Decisao::Aguardar("já existe um servidor respondendo");
    }
    let visiveis = outros.len() + 1;
    let total = total_conhecidos.max(visiveis);
    if visiveis * 2 <= total {
        return Decisao::Aguardar("sem quórum: não vejo a maioria dos computadores conhecidos");
    }
    let mut candidatos: Vec<&Membro> = outros
        .iter()
        .copied()
        .filter(|m| m.papel == "cliente" || m.papel == "sem_servidor")
        .collect();
    candidatos.push(eu);
    candidatos.sort_by(|a, b| precedencia(a).cmp(&precedencia(b)));
    if candidatos[0].dispositivo == eu.dispositivo {
        Decisao::Promover
    } else {
        Decisao::Aguardar("há computador com dados mais recentes ou mais antigo disponível")
    }
}

/// Decisão de quem já é servidor ao encontrar outros computadores (função pura).
///
/// Cede (vira cliente) se algum computador tem época maior que a sua: houve promoção depois dela,
/// então os dados mais recentes estão em outro lugar. Também cede se outro servidor tem precedência
/// pela mesma regra (época, ativação, identificador). Caso contrário, continua servidor.
pub fn decidir_servidor(eu: &Membro, outros: &[Membro]) -> Decisao {
    let outros = unicos(eu, outros);
    if outros.iter().any(|m| m.epoca > eu.epoca) {
        return Decisao::Ceder;
    }
    if outros.iter().any(|m| m.papel == "servidor" && precedencia(m) < precedencia(eu)) {
        return Decisao::Ceder;
    }
    Decisao::Manter
}

/// Época que um computador recebe ao virar servidor: uma unidade acima da maior época que ele conhece.
pub fn proxima_epoca(eu: &Membro, outros: &[Membro]) -> u64 {
    outros.iter().map(|m| m.epoca).chain(std::iter::once(eu.epoca)).max().unwrap_or(0) + 1
}

/// Um cliente só copia dados de um servidor com época igual ou mais recente que a sua.
/// Servidor atrasado (ex.: que voltou de uma queda) não serve os dados.
pub fn servidor_aceitavel(epoca_local: u64, epoca_servidor: u64) -> bool {
    epoca_servidor >= epoca_local
}

/// Validade de uma sessão de login na rede.
pub const VALIDADE_SESSAO: Duration = Duration::from_secs(10 * 60 * 60);

/// Sessão emitida pelo servidor no login. A identidade vem daqui, nunca do que o cliente informa em cada comando.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessaoRede {
    pub usuario: String,
    pub perfil: String,
    pub dispositivo: String,
    expira: Instant,
}

/// Tokens de sessão do servidor (só em memória: reiniciar o servidor exige novo login).
#[derive(Default)]
pub struct Sessoes {
    mapa: Mutex<HashMap<String, SessaoRede>>,
}

impl Sessoes {
    /// Emite um token novo (64 hex, aleatório) para o usuário, preso ao dispositivo que fez o login.
    pub fn emitir(&self, usuario: &str, perfil: &str, dispositivo: &str, agora: Instant) -> String {
        let token = format!("{}{}", hex::encode(nonce_aleatorio()), hex::encode(nonce_aleatorio()));
        let mut m = self.mapa.lock().unwrap_or_else(|e| e.into_inner());
        m.retain(|_, s| s.expira > agora);
        m.insert(
            token.clone(),
            SessaoRede {
                usuario: usuario.into(),
                perfil: perfil.into(),
                dispositivo: dispositivo.into(),
                expira: agora + VALIDADE_SESSAO,
            },
        );
        token
    }

    /// Devolve a sessão se o token existe, não expirou e foi usado pelo mesmo dispositivo.
    pub fn validar(&self, token: &str, dispositivo: &str, agora: Instant) -> Option<SessaoRede> {
        let m = self.mapa.lock().unwrap_or_else(|e| e.into_inner());
        let s = m.get(token)?;
        if s.expira <= agora || s.dispositivo != dispositivo {
            return None;
        }
        Some(s.clone())
    }

    pub fn encerrar(&self, token: &str) {
        self.mapa.lock().unwrap_or_else(|e| e.into_inner()).remove(token);
    }
}
