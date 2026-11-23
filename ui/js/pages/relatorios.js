// Relatórios de gestão (só leitura): faturamento, recebimentos, contas, lucro, serviços, produtividade e estoque.
// Escolha o período, veja os cartões, imprima ou exporte um arquivo de texto guardado no computador.
import { chamar } from '../api.js';
import { el, aviso, botao, icone, cabecalhoPagina, moeda } from '../components.js';

const FORMAS = [['dinheiro', 'Dinheiro'], ['pix', 'Pix'], ['cartao', 'Cartão'], ['outro', 'Outro']];

// Datas no fuso do computador, em AAAA-MM-DD.
const iso = (d) => `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
const br = (aaaaMmDd) => (aaaaMmDd ? aaaaMmDd.split('-').reverse().join('/') : '');

function periodoRapido(tipo) {
  const hoje = new Date();
  if (tipo === 'hoje') return { de: iso(hoje), ate: iso(hoje) };
  if (tipo === 'semana') {
    const dia = (hoje.getDay() + 6) % 7; // segunda = 0
    const inicio = new Date(hoje); inicio.setDate(hoje.getDate() - dia);
    const fim = new Date(inicio); fim.setDate(inicio.getDate() + 6);
    return { de: iso(inicio), ate: iso(fim) };
  }
  // este mês
  return { de: iso(new Date(hoje.getFullYear(), hoje.getMonth(), 1)), ate: iso(new Date(hoje.getFullYear(), hoje.getMonth() + 1, 0)) };
}

const real = (c) => moeda((c ?? 0) / 100);
const qtd = (n, s, p) => `${n} ${n === 1 ? s : p}`;

function cartao(titulo, ic, ...filhos) {
  return el('section', { class: 'card secao relatorio-cartao' }, el('h3', {}, icone(ic, 24), titulo), ...filhos);
}

function linhaTotal(rotulo, t) {
  return el('p', { class: 'texto-grande' }, `${rotulo}: ${real(t.centavos)} (${qtd(t.quantidade, 'atendimento', 'atendimentos')})`);
}

// Constrói os cartões a partir do relatório completo.
function cartoes(r) {
  const f = r.faturamento, rc = r.recebimentos, c = r.contas, l = r.lucro, s = r.servicos, e = r.estoque;
  const lista = (itens) => el('ul', { class: 'lista-simples' }, itens.map((t) => el('li', {}, t)));
  return [
    cartao('Faturamento', 'caixa',
      el('p', { class: 'ajuda' }, `Período: ${r.periodo}. Entra o que foi concluído no período.`),
      linhaTotal('Total', f.total),
      lista([
        `Ordens de serviço: ${real(f.servicos_os.centavos)} (${qtd(f.servicos_os.quantidade, 'ordem', 'ordens')})`,
        `Vendas de balcão: ${real(f.vendas_balcao.centavos)} (${qtd(f.vendas_balcao.quantidade, 'venda', 'vendas')})`,
      ])),
    cartao('Recebimentos por forma de pagamento', 'caixa',
      el('p', { class: 'ajuda' }, 'Valores recebidos, pela forma escolhida ao concluir cada ordem ou venda.'),
      lista([
        ...FORMAS.map(([k, nome]) => `${nome}: ${real(rc[k].centavos)} (${qtd(rc[k].quantidade, 'recebimento', 'recebimentos')})`),
      ].filter(Boolean)),
      linhaTotal('Total recebido', rc.total)),
    cartao('Contas em aberto e vencidas', 'alerta',
      el('p', { class: 'ajuda' }, `Situação em ${c.hoje}. Não depende do período.`),
      lista([
        `Ordens em aberto (criadas ou em andamento): ${real(c.ordens_em_aberto.centavos)} (${qtd(c.ordens_em_aberto.quantidade, 'ordem', 'ordens')})`,
        `Ordens com prazo já vencido: ${real(c.ordens_com_prazo_vencido.centavos)} (${qtd(c.ordens_com_prazo_vencido.quantidade, 'ordem', 'ordens')})`,
        `A receber (saldo): ${real(c.contas_a_receber.centavos)}, dos quais vencidos ${real(c.contas_a_receber_vencidas.centavos)}`,
        `A pagar (saldo): ${real(c.contas_a_pagar.centavos)}, dos quais vencidos ${real(c.contas_a_pagar_vencidas.centavos)}`,
      ]),
      el('p', { class: 'ajuda' }, c.observacao)),
    cartao('Lucro (resultado antes de despesas)', 'caixa',
      el('p', { class: 'texto-grande' }, `Resultado: ${real(l.resultado_centavos)}`),
      lista([
        `Receita: ${real(l.receita_centavos)}`,
        `Custo das peças vendidas: ${real(l.custo_pecas_centavos)}`,
        `Despesas do período: ${real(l.despesas_centavos)}`,
      ]),
      el('h4', {}, 'O que está incluído'), lista(l.incluido),
      el('h4', {}, 'O que NÃO está incluído'), lista(l.nao_incluido)),
    cartao('Serviços realizados', 'ordens',
      el('p', { class: 'texto-grande' }, `${qtd(s.quantidade, 'serviço', 'serviços')} lançados, somando ${real(s.centavos)}`),
      s.mais_realizados.length
        ? el('div', { class: 'tabela-wrap' }, el('table', {},
          el('thead', {}, el('tr', {}, el('th', {}, 'Serviço'), el('th', {}, 'Quantidade'), el('th', {}, 'Valor'))),
          el('tbody', {}, s.mais_realizados.map((x) => el('tr', {},
            el('td', {}, x.nome), el('td', {}, String(x.quantidade)), el('td', {}, real(x.centavos)))))))
        : el('p', { class: 'vazio' }, 'Nenhum serviço concluído neste período.')),
    cartao('Produtividade por responsável', 'equipe',
      el('p', { class: 'ajuda' }, 'Responsável pela OS é o funcionário que a abriu e responde por ela.'),
      r.produtividade.length
        ? el('div', { class: 'tabela-wrap' }, el('table', {},
          el('thead', {}, el('tr', {}, el('th', {}, 'Responsável pela OS'), el('th', {}, 'Ordens concluídas'), el('th', {}, 'Serviços lançados'), el('th', {}, 'Valor'))),
          el('tbody', {}, r.produtividade.map((p) => el('tr', {},
            el('td', {}, p.responsavel_nome), el('td', {}, String(p.ordens_concluidas)), el('td', {}, String(p.servicos_lancados)), el('td', {}, real(p.centavos)))))))
        : el('p', { class: 'vazio' }, 'Nenhuma ordem concluída neste período.')),
    cartao('Estoque', 'pecas',
      el('h4', {}, 'Peças com saldo baixo agora'),
      e.saldo_baixo.length
        ? lista(e.saldo_baixo.map((i) => `${i.nome}: ${i.quantidade_atual} em estoque (mínimo ${i.estoque_minimo})`))
        : el('p', { class: 'vazio' }, 'Nenhuma peça com saldo baixo.'),
      el('h4', {}, 'Saídas no período (peças usadas em ordens e vendas)'),
      e.saidas.length
        ? lista(e.saidas.map((x) => `${x.nome}: ${x.quantidade} unidade(s), ${real(x.centavos)}`))
        : el('p', { class: 'vazio' }, 'Nenhuma saída de peça neste período.'),
      el('p', { class: 'texto-grande' }, `Total de saídas: ${qtd(e.saidas_quantidade, 'unidade', 'unidades')}, ${real(e.saidas_centavos)}`),
      el('h4', {}, 'Entradas (compras lançadas no período)'),
      e.entradas.length
        ? lista(e.entradas.map((x) => `${x.nome}: ${x.quantidade} unidade(s), ${real(x.centavos)}`))
        : el('p', { class: 'vazio' }, 'Nenhuma compra de peças lançada no período.'),
      el('p', { class: 'texto-grande' }, `Total de entradas: ${qtd(e.entradas_quantidade, 'unidade', 'unidades')}, ${real(e.entradas_centavos)}`)),
  ];
}

// Texto simples para arquivo (ou para colar em outro programa). Não envia nada a serviço nenhum.
export function textoDoRelatorio(r) {
  const L = [];
  const h = (t) => { L.push('', t.toUpperCase(), '-'.repeat(t.length)); };
  L.push(`RELATÓRIO DA OFICINA - período ${r.periodo}`, `Gerado em ${r.gerado_em}`);
  const f = r.faturamento;
  h('Faturamento');
  L.push(`Total: ${real(f.total.centavos)} (${f.total.quantidade})`, `Ordens de serviço: ${real(f.servicos_os.centavos)} (${f.servicos_os.quantidade})`, `Vendas de balcão: ${real(f.vendas_balcao.centavos)} (${f.vendas_balcao.quantidade})`);
  h('Recebimentos por forma de pagamento');
  for (const [k, nome] of FORMAS) L.push(`${nome}: ${real(r.recebimentos[k].centavos)} (${r.recebimentos[k].quantidade})`);
  L.push(`Total recebido: ${real(r.recebimentos.total.centavos)}`);
  h(`Contas em aberto e vencidas (situação em ${r.contas.hoje})`);
  L.push(`Ordens em aberto: ${real(r.contas.ordens_em_aberto.centavos)} (${r.contas.ordens_em_aberto.quantidade})`, `Ordens com prazo vencido: ${real(r.contas.ordens_com_prazo_vencido.centavos)} (${r.contas.ordens_com_prazo_vencido.quantidade})`, `A receber: ${real(r.contas.contas_a_receber.centavos)} (vencidos ${real(r.contas.contas_a_receber_vencidas.centavos)})`, `A pagar: ${real(r.contas.contas_a_pagar.centavos)} (vencidos ${real(r.contas.contas_a_pagar_vencidas.centavos)})`, r.contas.observacao);
  h('Lucro (antes de despesas)');
  L.push(`Receita: ${real(r.lucro.receita_centavos)}`, `Custo das peças vendidas: ${real(r.lucro.custo_pecas_centavos)}`, `Despesas do período: ${real(r.lucro.despesas_centavos)}`, `Resultado: ${real(r.lucro.resultado_centavos)}`);
  L.push('Está incluído:', ...r.lucro.incluido.map((t) => `- ${t}`), 'NÃO está incluído:', ...r.lucro.nao_incluido.map((t) => `- ${t}`));
  h('Serviços realizados');
  L.push(`Quantidade: ${r.servicos.quantidade}. Valor: ${real(r.servicos.centavos)}`, ...r.servicos.mais_realizados.map((x) => `- ${x.nome}: ${x.quantidade} vez(es), ${real(x.centavos)}`));
  h('Produtividade (Responsável pela OS)');
  L.push(...(r.produtividade.length ? r.produtividade.map((p) => `${p.responsavel_nome}: ${p.ordens_concluidas} ordem(ns), ${p.servicos_lancados} serviço(s), ${real(p.centavos)}`) : ['Nenhuma ordem concluída no período.']));
  h('Estoque');
  L.push('Saldo baixo agora:', ...(r.estoque.saldo_baixo.length ? r.estoque.saldo_baixo.map((i) => `- ${i.nome}: ${i.quantidade_atual} (mínimo ${i.estoque_minimo})`) : ['- nenhuma']));
  L.push('Saídas no período:', ...(r.estoque.saidas.length ? r.estoque.saidas.map((x) => `- ${x.nome}: ${x.quantidade} unidade(s), ${real(x.centavos)}`) : ['- nenhuma']));
  L.push('Entradas no período (compras):', ...(r.estoque.entradas.length ? r.estoque.entradas.map((x) => `- ${x.nome}: ${x.quantidade} unidade(s), ${real(x.centavos)}`) : ['- nenhuma']));
  return L.join('\n') + '\n';
}

// Salva o texto no arquivo escolhido pelo usuário (diálogo de salvar, quando o navegador oferece; senão, download).
async function exportarTexto(r) {
  const nome = `relatorio-${r.de}-a-${r.ate}.txt`;
  const blob = new Blob([textoDoRelatorio(r)], { type: 'text/plain;charset=utf-8' });
  if (window.showSaveFilePicker) {
    try {
      const alvo = await window.showSaveFilePicker({ suggestedName: nome, types: [{ description: 'Texto', accept: { 'text/plain': ['.txt'] } }] });
      const w = await alvo.createWritable();
      await w.write(blob);
      await w.close();
      aviso('Relatório salvo no arquivo escolhido.');
      return;
    } catch (e) {
      if (e?.name === 'AbortError') return;
    }
  }
  const a = el('a', { href: URL.createObjectURL(blob), download: nome });
  document.body.append(a);
  a.click();
  a.remove();
  aviso('Relatório exportado. Veja a pasta de downloads do computador.');
}

export async function render(container) {
  const de = el('input', { type: 'date', 'aria-label': 'Data inicial' });
  const ate = el('input', { type: 'date', 'aria-label': 'Data final' });
  const area = el('div', { class: 'relatorios' });
  let atual = null;

  const rapidos = [['hoje', 'Hoje'], ['semana', 'Esta semana'], ['mes', 'Este mês']].map(([tipo, texto]) =>
    botao({ texto, tipo: 'secundario', onclick: () => {
      const p = periodoRapido(tipo);
      de.value = p.de; ate.value = p.ate;
      gerar();
    } }));
  const personalizado = botao({ texto: 'Personalizado', icone: 'lapis', tipo: 'secundario', onclick: () => {
    de.focus();
    aviso('Escolha as datas De e Até e toque em Ver relatório.');
  } });

  async function gerar() {
    if (!de.value || !ate.value) { aviso('Escolha as datas De e Até.', 'erro'); return; }
    area.replaceChildren(el('p', { class: 'ajuda' }, 'Gerando relatório...'));
    try {
      atual = await chamar('relatorio_completo', { de: de.value, ate: ate.value });
      area.replaceChildren(...cartoes(atual));
    } catch (e) {
      atual = null;
      area.replaceChildren();
      aviso(e.texto ?? 'Não foi possível gerar o relatório.', 'erro');
    }
  }

  const padrao = periodoRapido('mes');
  de.value = padrao.de; ate.value = padrao.ate;

  container.replaceChildren(
    cabecalhoPagina('Relatórios', 'registro', 'Veja como foi a oficina em um período. Os relatórios só leem os dados, não mudam nada.'),
    el('section', { class: 'card' },
      el('h3', {}, 'Escolha o período'),
      el('div', { class: 'acoes' }, rapidos, personalizado),
      el('div', { class: 'grade-campos' },
        el('label', { class: 'campo' }, el('span', { class: 'rotulo' }, 'De'), de),
        el('label', { class: 'campo' }, el('span', { class: 'rotulo' }, 'Até'), ate)),
      el('div', { class: 'acoes' },
        botao({ texto: 'Ver relatório', icone: 'lupa', tipo: 'primario', onclick: gerar }),
        botao({ texto: 'Imprimir', icone: 'disco', onclick: () => {
          if (!atual) { aviso('Gere o relatório antes de imprimir.', 'erro'); return; }
          window.print();
        } }),
        botao({ texto: 'Exportar em arquivo de texto', icone: 'disco', onclick: () => {
          if (!atual) { aviso('Gere o relatório antes de exportar.', 'erro'); return; }
          exportarTexto(atual);
        } }))),
    area);
  gerar();
}
