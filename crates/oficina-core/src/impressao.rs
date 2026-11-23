//! Impressão em impressora térmica de cupom (papel de 80 mm), usando o crate `escpos`.
//!
//! O cupom é montado pelo `escpos` em memória e depois enviado ao destino configurado:
//! - rede: `endereco:porta` (impressoras térmicas de rede, porta padrão 9100);
//! - arquivo: caminho de dispositivo, como `/dev/usb/lp0` (impressora USB ligada direto).
use std::io::Write;
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Mutex;
use std::time::Duration;

use escpos::driver::Driver;
use escpos::printer::Printer;
use escpos::printer_options::PrinterOptions;
use escpos::utils::{JustifyMode, PageCode, Protocol};

use crate::caixa::Relatorio;
use crate::error::{AppError, AppResult, FieldError};
use crate::ordens::Ordem;
use crate::orcamentos::{self, Orcamento};

/// Colunas de uma impressora de 80 mm na fonte normal.
pub const COLUNAS: usize = 48;
const TEMPO: Duration = Duration::from_secs(5);

#[derive(Debug, PartialEq)]
pub enum Destino {
    Rede(String),
    Arquivo(String),
}

/// Entende o texto digitado em Configurações: `host:porta` ou caminho que começa com `/`.
pub fn destino(texto: &str) -> AppResult<Destino> {
    let t = texto.trim();
    let invalido = || AppError::Validation(vec![FieldError {
        field: "destino".into(),
        message: "Digite o endereço da impressora (ex.: 192.168.0.50:9100) ou o caminho (ex.: /dev/usb/lp0).".into(),
    }]);
    if t.is_empty() {
        return Err(invalido());
    }
    if t.starts_with('/') {
        return Ok(Destino::Arquivo(t.to_string()));
    }
    match t.rsplit_once(':') {
        Some((host, porta)) if !host.is_empty() && porta.parse::<u16>().is_ok() => Ok(Destino::Rede(t.to_string())),
        _ => Err(invalido()),
    }
}

/// Driver que guarda os bytes em memória. Assim o `escpos` monta o cupom sem conhecer o transporte.
#[derive(Default)]
struct Memoria(Mutex<Vec<u8>>);

impl Driver for Memoria {
    fn name(&self) -> String {
        "memoria".into()
    }
    fn write(&self, data: &[u8]) -> escpos::errors::Result<()> {
        self.0.lock().map_err(|_| escpos::errors::PrinterError::Io("buffer travado".into()))?.extend_from_slice(data);
        Ok(())
    }
    fn read(&self, _buf: &mut [u8]) -> escpos::errors::Result<usize> {
        Ok(0)
    }
    fn flush(&self) -> escpos::errors::Result<()> {
        Ok(())
    }
}

fn impressora() -> Printer<Memoria> {
    let opcoes = PrinterOptions::new(Some(PageCode::PC860), None, COLUNAS as u8);
    Printer::new(Memoria::default(), Protocol::default(), Some(opcoes))
}

fn recolher(p: Printer<Memoria>) -> AppResult<Vec<u8>> {
    let bytes = p.driver().0.into_inner().map_err(|_| AppError::Database("buffer da impressora travado".into()))?;
    Ok(bytes)
}

fn erro_cupom(e: escpos::errors::PrinterError) -> AppError {
    AppError::Conflict(format!("Não consegui montar o cupom: {e}"))
}

/// Envia os bytes ao destino. Erros viram mensagem clara para a tela.
pub fn enviar(dest: &Destino, bytes: &[u8]) -> AppResult<()> {
    match dest {
        Destino::Rede(endereco) => {
            let falha = |e: String| AppError::Conflict(format!("Não consegui falar com a impressora em {endereco}: {e}"));
            let addr = endereco.to_socket_addrs().map_err(|e| falha(e.to_string()))?.next()
                .ok_or_else(|| falha("endereço não encontrado".into()))?;
            let mut con = TcpStream::connect_timeout(&addr, TEMPO).map_err(|e| falha(e.to_string()))?;
            con.set_write_timeout(Some(TEMPO)).map_err(|e| falha(e.to_string()))?;
            con.write_all(bytes).map_err(|e| falha(e.to_string()))?;
            con.flush().map_err(|e| falha(e.to_string()))?;
            Ok(())
        }
        Destino::Arquivo(caminho) => {
            let falha = |e: String| AppError::Conflict(format!("Não consegui escrever na impressora em {caminho}: {e}"));
            let mut arq = std::fs::OpenOptions::new().write(true).open(caminho).map_err(|e| falha(e.to_string()))?;
            arq.write_all(bytes).map_err(|e| falha(e.to_string()))?;
            arq.flush().map_err(|e| falha(e.to_string()))?;
            Ok(())
        }
    }
}

/// Bytes de uma página de teste: confirma que a impressora responde.
pub fn teste() -> AppResult<Vec<u8>> {
    let mut p = impressora();
    p.init().map_err(erro_cupom)?;
    p.justify(JustifyMode::CENTER).map_err(erro_cupom)?
        .bold(true).map_err(erro_cupom)?
        .writeln("TESTE DE IMPRESSAO").map_err(erro_cupom)?
        .bold(false).map_err(erro_cupom)?
        .writeln("Se este texto saiu, a impressora esta ligada.").map_err(erro_cupom)?
        .feeds(3).map_err(erro_cupom)?
        .print_cut().map_err(erro_cupom)?;
    recolher(p)
}

/// Cupom da ordem de serviço.
pub fn cupom_os(os: &Ordem) -> AppResult<Vec<u8>> {
    let mut p = impressora();
    p.init().map_err(erro_cupom)?;
    p.justify(JustifyMode::CENTER).map_err(erro_cupom)?
        .bold(true).map_err(erro_cupom)?
        .size(2, 2).map_err(erro_cupom)?
        .writeln("OFICINA").map_err(erro_cupom)?
        .size(1, 1).map_err(erro_cupom)?
        .bold(false).map_err(erro_cupom)?
        .writeln(&format!("Ordem de servico No {}", os.id)).map_err(erro_cupom)?
        .writeln(&format!("Entrada: {}", data_curta(&os.data_entrada))).map_err(erro_cupom)?;
    p.justify(JustifyMode::LEFT).map_err(erro_cupom)?;
    p.writeln(&"-".repeat(COLUNAS)).map_err(erro_cupom)?;
    p.writeln(&format!("Veiculo: {} {}", os.veiculo_placa, os.veiculo_modelo)).map_err(erro_cupom)?;
    p.writeln(&format!("Cliente: {}", os.cliente_nome)).map_err(erro_cupom)?;
    p.writeln(&format!("Responsavel: {}", os.funcionario_nome)).map_err(erro_cupom)?;
    if let Some(d) = os.descricao.as_deref().filter(|d| !d.trim().is_empty()) {
        for parte in quebrar(&format!("Problema: {d}"), COLUNAS) {
            p.writeln(&parte).map_err(erro_cupom)?;
        }
    }
    p.writeln(&"-".repeat(COLUNAS)).map_err(erro_cupom)?;
    p.writeln(&format!("{:<22} {:>3} {:>10} {:>10}", "Item", "Qtd", "Unit.", "Subtotal")).map_err(erro_cupom)?;
    for l in &os.itens {
        let nome: String = l.item_nome.chars().take(22).collect();
        p.writeln(&format!("{:<22} {:>3} {:>10} {:>10}", nome, l.quantidade, moeda(l.preco_unitario), moeda(l.subtotal)))
            .map_err(erro_cupom)?;
    }
    p.writeln(&"-".repeat(COLUNAS)).map_err(erro_cupom)?;
    p.justify(JustifyMode::RIGHT).map_err(erro_cupom)?
        .bold(true).map_err(erro_cupom)?
        .writeln(&format!("TOTAL {}", moeda(os.valor_total))).map_err(erro_cupom)?
        .bold(false).map_err(erro_cupom)?
        .justify(JustifyMode::CENTER).map_err(erro_cupom)?
        .feeds(1).map_err(erro_cupom)?;
    // Código de barras com o número da ordem: serve para localizar a OS depois.
    p.code39(&format!("OS{:06}", os.id)).map_err(erro_cupom)?
        .feeds(4).map_err(erro_cupom)?
        .print_cut().map_err(erro_cupom)?;
    recolher(p)
}

/// Cupom de orçamento (ou de aditivo). Vem marcado como ORCAMENTO e avisa que não é documento fiscal.
pub fn cupom_orcamento(o: &Orcamento) -> AppResult<Vec<u8>> {
    let traco = "-".repeat(COLUNAS);
    let titulo = if o.tipo == orcamentos::TIPO_ADITIVO {
        format!("Adicional da OS No {}", o.ordem_id.unwrap_or(0))
    } else {
        format!("Orcamento No {}", o.id)
    };
    let mut p = impressora();
    p.init().map_err(erro_cupom)?;
    p.justify(JustifyMode::CENTER).map_err(erro_cupom)?
        .bold(true).map_err(erro_cupom)?
        .size(2, 2).map_err(erro_cupom)?
        .writeln("OFICINA").map_err(erro_cupom)?
        .size(1, 1).map_err(erro_cupom)?
        .writeln("ORCAMENTO").map_err(erro_cupom)?
        .bold(false).map_err(erro_cupom)?
        .writeln("NAO E DOCUMENTO FISCAL").map_err(erro_cupom)?
        .writeln(&titulo).map_err(erro_cupom)?;
    p.justify(JustifyMode::LEFT).map_err(erro_cupom)?;
    p.writeln(&traco).map_err(erro_cupom)?;
    p.writeln(&format!("Veiculo: {} {}", o.veiculo_placa, o.veiculo_modelo)).map_err(erro_cupom)?;
    p.writeln(&format!("Cliente: {}", o.cliente_nome)).map_err(erro_cupom)?;
    p.writeln(&format!("Responsavel: {}", o.funcionario_nome)).map_err(erro_cupom)?;
    p.writeln(&format!("Emitido em: {}", data_curta(&o.criado_em))).map_err(erro_cupom)?;
    p.writeln(&format!("Situacao: {}", situacao_impressa(o.status))).map_err(erro_cupom)?;
    if let Some(aprovado) = o.aprovado_em.as_deref() {
        let forma = o.aprovacao_forma.as_deref().unwrap_or("-");
        p.writeln(&format!("Aprovacao informada em {} ({})", data_curta(aprovado), forma)).map_err(erro_cupom)?;
        if let Some(obs) = o.aprovacao_obs.as_deref().filter(|s| !s.trim().is_empty()) {
            for parte in quebrar(&format!("Obs.: {obs}"), COLUNAS) {
                p.writeln(&parte).map_err(erro_cupom)?;
            }
        }
    }
    p.writeln(&traco).map_err(erro_cupom)?;
    p.writeln(&format!("{:<22} {:>3} {:>10} {:>10}", "Item", "Qtd", "Unit.", "Subtotal")).map_err(erro_cupom)?;
    for l in &o.itens {
        let nome: String = l.nome.chars().take(22).collect();
        p.writeln(&format!(
            "{:<22} {:>3} {:>10} {:>10}",
            nome,
            l.quantidade,
            moeda(l.preco_unitario_centavos as f64 / 100.0),
            moeda(l.subtotal_centavos as f64 / 100.0)
        ))
        .map_err(erro_cupom)?;
    }
    p.writeln(&traco).map_err(erro_cupom)?;
    p.justify(JustifyMode::RIGHT).map_err(erro_cupom)?
        .bold(true).map_err(erro_cupom)?
        .writeln(&format!("TOTAL {}", moeda(o.valor_total_centavos as f64 / 100.0))).map_err(erro_cupom)?
        .bold(false).map_err(erro_cupom)?;
    p.justify(JustifyMode::LEFT).map_err(erro_cupom)?;
    if let Some(d) = o.prazo_dias {
        p.writeln(&format!("Prazo estimado do servico: {d} dias")).map_err(erro_cupom)?;
    }
    if let Some(v) = o.validade.as_deref() {
        p.writeln(&format!("Valores validos ate: {}", data_br(v))).map_err(erro_cupom)?;
    }
    if let Some(obs) = o.observacoes.as_deref().filter(|s| !s.trim().is_empty()) {
        for parte in quebrar(&format!("Obs.: {obs}"), COLUNAS) {
            p.writeln(&parte).map_err(erro_cupom)?;
        }
    }
    p.writeln(&traco).map_err(erro_cupom)?;
    p.justify(JustifyMode::CENTER).map_err(erro_cupom)?
        .writeln("Este papel e apenas um orcamento.").map_err(erro_cupom)?
        .writeln("Nao vale como nota fiscal.").map_err(erro_cupom)?
        .feeds(4).map_err(erro_cupom)?
        .print_cut().map_err(erro_cupom)?;
    recolher(p)
}

/// Situação em texto simples (sem acento, como os demais cupons).
fn situacao_impressa(s: orcamentos::Status) -> &'static str {
    match s {
        orcamentos::Status::Rascunho => "Rascunho, ainda nao enviado",
        orcamentos::Status::AguardandoAprovacao => "Aguardando aprovacao do cliente",
        orcamentos::Status::Aprovado => "Aprovado",
        orcamentos::Status::Recusado => "Recusado pelo cliente",
        orcamentos::Status::Cancelado => "Cancelado",
    }
}

/// AAAA-MM-DD para DD/MM/AAAA (usado na validade).
fn data_br(iso: &str) -> String {
    chrono::NaiveDate::parse_from_str(iso, "%Y-%m-%d")
        .map(|d| d.format("%d/%m/%Y").to_string())
        .unwrap_or_else(|_| iso.to_string())
}

/// Cupom de fechamento de caixa: gaveta, recebimentos por forma, movimentos e ordens recebidas.
pub fn cupom_fechamento(r: &Relatorio) -> AppResult<Vec<u8>> {
    let s = &r.sessao;
    let traco = "-".repeat(COLUNAS);
    let mut p = impressora();
    p.init().map_err(erro_cupom)?;
    p.justify(JustifyMode::CENTER).map_err(erro_cupom)?
        .bold(true).map_err(erro_cupom)?
        .size(2, 2).map_err(erro_cupom)?
        .writeln("OFICINA").map_err(erro_cupom)?
        .size(1, 1).map_err(erro_cupom)?
        .bold(false).map_err(erro_cupom)?
        .writeln(&format!("Fechamento de caixa No {}", s.id)).map_err(erro_cupom)?;
    p.justify(JustifyMode::LEFT).map_err(erro_cupom)?;
    p.writeln(&traco).map_err(erro_cupom)?;
    p.writeln(&format!("Aberto:   {} por {}", data_curta(&s.aberto_em), s.aberto_por)).map_err(erro_cupom)?;
    let fechou = s.fechado_em.as_deref().map(data_curta).unwrap_or_else(|| "em aberto".into());
    p.writeln(&format!("Fechado:  {} por {}", fechou, s.fechado_por.as_deref().unwrap_or("-"))).map_err(erro_cupom)?;
    p.writeln(&traco).map_err(erro_cupom)?;
    linha_valor(&mut p, "Troco inicial", s.valor_abertura)?;
    linha_valor(&mut p, "Recebido em dinheiro", r.recebimentos.dinheiro)?;
    linha_valor(&mut p, "Recebido em PIX", r.recebimentos.pix)?;
    linha_valor(&mut p, "Recebido em cartao", r.recebimentos.cartao)?;
    linha_valor(&mut p, "Recebido em outra forma", r.recebimentos.outro)?;
    linha_valor(&mut p, "Total recebido", r.recebimentos.total)?;
    linha_valor(&mut p, "Suprimentos", r.suprimentos)?;
    linha_valor(&mut p, "Sangrias", r.sangrias)?;
    p.writeln(&traco).map_err(erro_cupom)?;
    linha_valor(&mut p, "Troco inicial", r.sessao.valor_abertura)?;
    linha_valor(&mut p, "Dinheiro esperado", r.dinheiro_esperado)?;
    if let Some(c) = r.dinheiro_contado {
        linha_valor(&mut p, "Dinheiro contado", c)?;
    }
    if let Some(d) = r.diferenca {
        linha_valor(&mut p, "Diferenca", d)?;
    }
    p.writeln(&traco).map_err(erro_cupom)?;
    p.writeln(&format!("Atendimentos: {}", r.recebimentos.atendimentos)).map_err(erro_cupom)?;
    p.writeln(&traco).map_err(erro_cupom)?;
    p.writeln("Vendas e ordens recebidas:").map_err(erro_cupom)?;
    for o in &r.recebidos {
        let tipo = if o.tipo == "BALCAO" { "Venda" } else { "OS" };
        let cliente: String = o.cliente.chars().take(COLUNAS - 22).collect();
        p.writeln(&format!("{} {:<6} {}", tipo, o.os_id, cliente)).map_err(erro_cupom)?;
        p.writeln(&format!("  Total {} | recebido {} | {}", moeda(o.total_venda), moeda(o.valor), forma_nome(&o.pagamento)))
            .map_err(erro_cupom)?;
    }
    if !r.movimentos.is_empty() {
        p.writeln(&traco).map_err(erro_cupom)?;
        p.writeln("Movimentos da gaveta:").map_err(erro_cupom)?;
        for m in &r.movimentos {
            let tipo = if m.tipo == "sangria" { "Sangria" } else { "Suprimento" };
            let texto = format!("{} {} {} - {} ({})", data_curta(&m.criado_em), tipo, moeda(m.valor), m.motivo, m.usuario);
            for parte in quebrar(&texto, COLUNAS) {
                p.writeln(&parte).map_err(erro_cupom)?;
            }
        }
    }
    if let Some(obs) = s.observacao.as_deref().filter(|o| !o.trim().is_empty()) {
        for parte in quebrar(&format!("Obs: {obs}"), COLUNAS) {
            p.writeln(&parte).map_err(erro_cupom)?;
        }
    }
    p.writeln(&traco).map_err(erro_cupom)?;
    p.justify(JustifyMode::CENTER).map_err(erro_cupom)?
        .feeds(4).map_err(erro_cupom)?
        .print_cut().map_err(erro_cupom)?;
    recolher(p)
}

fn linha_valor(p: &mut Printer<Memoria>, rotulo: &str, valor: f64) -> AppResult<()> {
    p.writeln(&format!("{:<30}{:>18}", rotulo, moeda(valor))).map_err(erro_cupom)?;
    Ok(())
}

fn forma_nome(forma: &str) -> &'static str {
    match forma {
        "dinheiro" => "Dinheiro",
        "pix" => "PIX",
        "cartao" => "Cartao",
        _ => "Outro",
    }
}

fn moeda(v: f64) -> String {
    format!("R$ {:.2}", v).replace('.', ",")
}

fn data_curta(texto: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(texto)
        .map(|d| d.format("%d/%m/%Y %H:%M").to_string())
        .unwrap_or_else(|_| texto.chars().take(16).collect())
}

fn quebrar(texto: &str, largura: usize) -> Vec<String> {
    let mut linhas = Vec::new();
    let mut atual = String::new();
    for palavra in texto.split_whitespace() {
        if !atual.is_empty() && atual.chars().count() + 1 + palavra.chars().count() > largura {
            linhas.push(std::mem::take(&mut atual));
        }
        if !atual.is_empty() {
            atual.push(' ');
        }
        atual.push_str(palavra);
    }
    if !atual.is_empty() {
        linhas.push(atual);
    }
    linhas
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn destino_aceita_rede_e_arquivo() {
        assert_eq!(destino("192.168.0.50:9100").unwrap(), Destino::Rede("192.168.0.50:9100".into()));
        assert_eq!(destino("/dev/usb/lp0").unwrap(), Destino::Arquivo("/dev/usb/lp0".into()));
        assert!(destino("").is_err());
        assert!(destino("192.168.0.50").is_err());
        assert!(destino("host:porta_ruim").is_err());
    }

    #[test]
    fn quebra_respeita_largura() {
        let partes = quebrar("uma frase bem comprida para quebrar em linhas curtas", 16);
        assert!(partes.iter().all(|p| p.chars().count() <= 16));
        assert_eq!(partes.join(" "), "uma frase bem comprida para quebrar em linhas curtas");
    }

    #[test]
    fn teste_gera_comandos_de_corte_e_init() {
        let bytes = teste().unwrap();
        // ESC @ (inicializar) no começo e corte GS V no fim.
        assert_eq!(&bytes[..2], &[0x1B, 0x40]);
        assert!(bytes.windows(2).any(|w| w == [0x1D, 0x56]));
    }
}
