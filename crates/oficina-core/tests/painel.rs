// Painel do administrador com um cenário pequeno. Os valores esperados foram conferidos à mão (centavos).
//
// Período: 01/11/2026 a 07/11/2026. Horários às 12h UTC (mesmo dia local no Brasil).
//  OS 1      concluída 02/11, valor 80,00                            -> dia 02
//  OS 2      aguardando pagamento 02/11, valor 230,00                -> dia 02 (dia 02 total: 310,00 em 2 ordens)
//  Venda 3   concluída 05/11, balcão, valor 30,00                    -> dia 05
//  OS 6      concluída 30/10 (fora do período), valor 999,00         -> não entra
//  Recebimentos no período: pix 80,00 (OS 1), dinheiro 100,00 (OS 2), cartão 230,00 (OS 2), outro 15,00 (venda 3)
//  Recebimento de 31/10 (pix 999,00) -> fora do período
//  Dias sem venda: 01, 03, 04, 06, 07.
use oficina_core::comandos::{executar, Contexto};
use oficina_core::db;
use oficina_core::painel::{self, ContasResumo, DiaFaturamento, EstoqueBaixo, FormaRecebimento, ServicoTop, SituacaoOs};
use rusqlite::Connection;
use serde_json::json;

fn cenario() -> Connection {
    let conn = db::open_in_memory().unwrap();
    conn.execute_batch(
        "INSERT INTO clientes (id, nome) VALUES (1, 'Ana');
         INSERT INTO veiculos (id, cliente_id, placa, modelo) VALUES (1, 1, 'ABC1D23', 'Polo');
         INSERT INTO funcionarios (id, nome) VALUES (1, 'Carlos');
         INSERT INTO itens (id, nome, tipo, preco_custo, preco_venda, quantidade, estoque_minimo) VALUES
           (1, 'Filtro de óleo', 'peca', 10, 30, 5, 2),
           (2, 'Pastilha', 'peca', 40, 80, 1, 2),
           (3, 'Troca de óleo', 'servico', 0, 50, 0, 0),
           (4, 'Alinhamento', 'servico', 0, 70, 0, 0),
           (5, 'Lavagem', 'servico', 0, 20, 0, 0),
           (6, 'Balanceamento', 'servico', 0, 40, 0, 0),
           (7, 'Revisão', 'servico', 0, 100, 0, 0),
           (8, 'Polimento', 'servico', 0, 60, 0, 0),
           (9, 'Óleo 5W30', 'peca', 20, 45, 2, 2),
           (10, 'Vela', 'peca', 5, 15, 10, 3);
         INSERT INTO ordens_servico (id, tipo, veiculo_id, cliente_id, funcionario_id, status, data_entrada, data_conclusao, valor_total, pagamento) VALUES
           (1, 'OS', 1, NULL, 1, 'CONCLUIDA', '2026-11-01T09:00:00+00:00', '2026-11-02T12:00:00+00:00', 80, 'pix'),
           (2, 'OS', 1, NULL, 1, 'AGUARDANDO_PAGAMENTO', '2026-11-02T09:00:00+00:00', '2026-11-02T12:00:00+00:00', 230, NULL),
           (3, 'BALCAO', NULL, 1, 1, 'CONCLUIDA', '2026-11-05T12:00:00+00:00', '2026-11-05T12:00:00+00:00', 30, 'outro'),
           (4, 'OS', 1, NULL, 1, 'CANCELADA', '2026-11-03T09:00:00+00:00', NULL, 300, NULL),
           (5, 'OS', 1, NULL, 1, 'CRIADA', '2026-11-03T09:00:00+00:00', NULL, 50, NULL),
           (6, 'OS', 1, NULL, 1, 'CONCLUIDA', '2026-10-28T09:00:00+00:00', '2026-10-30T12:00:00+00:00', 999, 'pix'),
           (7, 'OS', 1, NULL, 1, 'EM_ANDAMENTO', '2026-11-04T09:00:00+00:00', NULL, 40, NULL),
           (8, 'OS', 1, NULL, 1, 'CRIADA', '2026-11-08T09:00:00+00:00', NULL, 10, NULL);
         INSERT INTO ordem_itens (os_id, item_id, quantidade, preco_unitario, subtotal) VALUES
           (1, 3, 1, 50, 50), (1, 4, 1, 70, 70), (1, 6, 2, 40, 80), (1, 8, 1, 60, 60),
           (2, 3, 2, 50, 100), (2, 5, 4, 20, 80), (2, 7, 5, 100, 500),
           (3, 1, 1, 30, 30),
           (4, 7, 3, 100, 300),
           (5, 6, 9, 40, 360),
           (6, 7, 10, 100, 1000),
           (7, 5, 2, 20, 40);
         INSERT INTO recebimentos (ordem_id, valor_centavos, forma, data, usuario) VALUES
           (1, 8000, 'pix', '2026-11-02T12:00:00+00:00', 'Carlos'),
           (2, 10000, 'dinheiro', '2026-11-02T13:00:00+00:00', 'Carlos'),
           (2, 23000, 'cartao', '2026-11-05T12:00:00+00:00', 'Carlos'),
           (3, 1500, 'outro', '2026-11-05T12:00:00+00:00', 'Carlos'),
           (6, 99900, 'pix', '2026-10-31T12:00:00+00:00', 'Carlos');
         INSERT INTO contas_receber (id, origem, referencia, cliente_id, descricao, valor_total_centavos, saldo_centavos, vencimento, data) VALUES
           (1, 'OS', 1, 1, 'OS 1', 5000, 5000, '2000-01-10', '2000-01-01'),
           (2, 'VENDA', 3, 1, 'Venda 3', 2000, 2000, '2099-12-31', '2000-01-01'),
           (3, 'MANUAL', NULL, 1, 'Quitada', 1000, 0, '2000-01-01', '2000-01-01');
         INSERT INTO fornecedores (id, nome, documento) VALUES (1, 'Peças SA', '00000000000191');
         INSERT INTO compras (id, fornecedor_id, data, total_centavos, pagamento, paga, criado_em)
           VALUES (1, 1, '2000-02-01', 20000, 'a pagar', 0, '2000-02-01T10:00:00+00:00');
         INSERT INTO contas_pagar (id, descricao, compra_id, valor_centavos, vencimento, data, usuario) VALUES
           (1, 'Aluguel', NULL, 50000, '2000-02-05', '2000-02-01T10:00:00+00:00', 'Carlos'),
           (2, 'Luz', NULL, 7000, '2099-01-10', '2026-11-02T10:00:00+00:00', 'Carlos'),
           (3, 'Internet', NULL, 3000, '2000-01-01', '2000-01-01T10:00:00+00:00', 'Carlos'),
           (4, 'Compra #1', 1, 20000, '2000-03-01', '2000-02-01', 'Carlos');
         INSERT INTO pagamentos_pagar (conta_id, valor_centavos, data, usuario) VALUES
           (1, 10000, '2000-02-03T10:00:00+00:00', 'Carlos'),
           (3, 3000, '2000-01-02T10:00:00+00:00', 'Carlos');",
    )
    .unwrap();
    conn
}

fn painel_nov() -> painel::Painel {
    painel::serie(&cenario(), "2026-11-01", "2026-11-07").unwrap()
}

fn dia(p: &painel::Painel, data: &str) -> DiaFaturamento {
    p.faturamento_por_dia.iter().find(|d| d.dia == data).cloned().unwrap()
}

#[test]
fn dias_sem_venda_aparecem_com_zero() {
    let p = painel_nov();
    // Sete dias, do 1º ao 7º, mesmo os que não tiveram venda.
    assert_eq!(p.faturamento_por_dia.len(), 7);
    assert_eq!(p.faturamento_por_dia[0].dia, "2026-11-01");
    assert_eq!(p.faturamento_por_dia[6].dia, "2026-11-07");
    assert_eq!(dia(&p, "2026-11-01"), DiaFaturamento { dia: "2026-11-01".into(), total_centavos: 0, quantidade: 0 });
    assert_eq!(dia(&p, "2026-11-04"), DiaFaturamento { dia: "2026-11-04".into(), total_centavos: 0, quantidade: 0 });
    // Dia 02: OS 1 (80,00) e OS 2 aguardando pagamento (230,00). Dia 05: venda 3 (30,00).
    assert_eq!(dia(&p, "2026-11-02"), DiaFaturamento { dia: "2026-11-02".into(), total_centavos: 31_000, quantidade: 2 });
    assert_eq!(dia(&p, "2026-11-05"), DiaFaturamento { dia: "2026-11-05".into(), total_centavos: 3_000, quantidade: 1 });
    // OS 6 (30/10) e OS cancelada/criada/em andamento não contam como faturamento.
    let total: i64 = p.faturamento_por_dia.iter().map(|d| d.total_centavos).sum();
    assert_eq!(total, 34_000);
}

#[test]
fn formas_de_recebimento_somam_certo() {
    let p = painel_nov();
    assert_eq!(
        p.recebimentos_por_forma,
        vec![
            FormaRecebimento { forma: "dinheiro".into(), total_centavos: 10_000 },
            FormaRecebimento { forma: "pix".into(), total_centavos: 8_000 },
            FormaRecebimento { forma: "cartao".into(), total_centavos: 23_000 },
            FormaRecebimento { forma: "outro".into(), total_centavos: 1_500 },
        ]
    );
}

#[test]
fn formas_sem_recebimento_aparecem_com_zero() {
    let conn = cenario();
    // Período sem nenhum recebimento: as quatro formas continuam no resultado, com zero.
    let p = painel::serie(&conn, "2026-12-01", "2026-12-31").unwrap();
    assert_eq!(p.recebimentos_por_forma.len(), 4);
    assert!(p.recebimentos_por_forma.iter().all(|f| f.total_centavos == 0));
}

#[test]
fn contas_separam_vencidas_do_total() {
    let p = painel_nov();
    // Hoje é depois de 2000 e antes de 2099, então: 2000 = vencido; 2099 = a vencer.
    // A receber: OS 1 (50,00, vencida) + venda 3 (20,00, a vencer). A conta quitada não entra.
    // A pagar: aluguel (500,00 − 100,00 pagos = 400,00, vencido); luz (70,00, a vencer);
    //          internet quitada (0); compra a prazo (200,00, vencida), contada uma única vez em contas_pagar.
    assert_eq!(
        p.contas,
        ContasResumo { receber_aberto: 7_000, receber_vencido: 5_000, pagar_aberto: 67_000, pagar_vencido: 60_000 }
    );
}

#[test]
fn servicos_top_5_ordenados_por_quantidade() {
    let p = painel_nov();
    // Revisão: 5 (OS 2). Lavagem: 4 (OS 2). Troca de óleo: 1 + 2 = 3. Balanceamento: 2 (OS 1).
    // Alinhamento e Polimento têm 1: empate por nome, então só Alinhamento entra no top 5.
    // Polimento fica de fora pelo limite. A OS 4 (cancelada), a 5 (criada), a 7 (em andamento) e a 6 (fora do período) não contam.
    assert_eq!(
        p.servicos_mais_realizados,
        vec![
            ServicoTop { nome: "Revisão".into(), quantidade: 5 },
            ServicoTop { nome: "Lavagem".into(), quantidade: 4 },
            ServicoTop { nome: "Troca de óleo".into(), quantidade: 3 },
            ServicoTop { nome: "Balanceamento".into(), quantidade: 2 },
            ServicoTop { nome: "Alinhamento".into(), quantidade: 1 },
        ]
    );
}

#[test]
fn estoque_baixo_mostra_so_pecas_no_minimo_ou_abaixo() {
    let p = painel_nov();
    // Pastilha (1 de mínimo 2) e Óleo 5W30 (2 de mínimo 2, igual ao mínimo). Filtro (5) e Vela (10) estão bem.
    // Serviços (quantidade 0 e mínimo 0) não entram, pois não têm estoque.
    assert_eq!(
        p.estoque_baixo,
        vec![
            EstoqueBaixo { nome: "Pastilha".into(), quantidade: 1, estoque_minimo: 2 },
            EstoqueBaixo { nome: "Óleo 5W30".into(), quantidade: 2, estoque_minimo: 2 },
        ]
    );
}

#[test]
fn os_por_situacao_conta_as_ordens_abertas_no_periodo() {
    let p = painel_nov();
    // Ordens com entrada de 01/11 a 07/11: 1, 2, 3, 4, 5 e 7. A 6 (outubro) e a 8 (dia 08) ficam de fora.
    assert_eq!(
        p.os_por_situacao,
        vec![
            SituacaoOs { situacao: "CRIADA".into(), quantidade: 1 },
            SituacaoOs { situacao: "EM_ANDAMENTO".into(), quantidade: 1 },
            SituacaoOs { situacao: "AGUARDANDO_PAGAMENTO".into(), quantidade: 1 },
            SituacaoOs { situacao: "CONCLUIDA".into(), quantidade: 2 },
            SituacaoOs { situacao: "CANCELADA".into(), quantidade: 1 },
        ]
    );
}

#[test]
fn periodo_invertido_e_recusado() {
    let conn = cenario();
    assert!(painel::serie(&conn, "2026-11-07", "2026-11-01").is_err());
}

#[test]
fn comando_painel_serie_so_para_administrador() {
    let conn = cenario();
    let admin = Contexto { usuario: "Carlos".into(), perfil: "administrador".into(), computador: "PC1".into() };
    let func = Contexto { usuario: "Ana".into(), perfil: "funcionario".into(), computador: "PC1".into() };
    let args = json!({ "de": "2026-11-01", "ate": "2026-11-07" });
    let valor = executar(&conn, "painel_serie", &args, &admin).unwrap();
    assert_eq!(valor["faturamento_por_dia"].as_array().unwrap().len(), 7);
    assert!(executar(&conn, "painel_serie", &args, &func).is_err());
}
