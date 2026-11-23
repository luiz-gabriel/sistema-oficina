// Tela inicial: pergunta principal, situação do caixa e blocos grandes para cada tarefa (com atalho de teclado).
import { chamar } from '../api.js';
import { el, icone, moeda, botao, tabela } from '../components.js';

const dataHora = (texto) => (texto ? new Date(texto).toLocaleString('pt-BR') : '');

// Administrador: mostra o caixa aberto com todos os valores e as vendas recebidas nele.
async function painelCaixasAbertos(area) {
  area.replaceChildren(el('p', { class: 'ajuda' }, 'Carregando...'));
  try {
    const s = await chamar('caixa_situacao');
    if (!s.aberto) {
      area.replaceChildren(el('p', { class: 'texto-grande', role: 'status' }, 'Nenhum caixa está aberto agora.'));
      return;
    }
    const rel = await chamar('caixa_relatorio', { id: s.sessao.id });
    const r = rel.recebimentos;
    const vendas = rel.recebidos.map((o) => ({
      tipo_texto: o.tipo === 'BALCAO' ? 'Venda de balcão' : 'Ordem de serviço',
      ordem: `${o.tipo === 'BALCAO' ? 'Venda' : 'OS'} nº ${o.os_id}`,
      cliente_texto: o.cliente || '—',
      total_texto: moeda(o.total_venda),
      recebido_texto: moeda(o.valor),
      forma: (o.pagamento || '').split(',').map((f) => f.trim()).join(', ') || '—',
    }));
    const indicadores = [
      ['Troco inicial', moeda(rel.sessao.valor_abertura)],
      ['Dinheiro recebido', moeda(r.dinheiro)],
      ['PIX', moeda(r.pix)],
      ['Cartão', moeda(r.cartao)],
      ['Outra forma', moeda(r.outro)],
      ['Total recebido', moeda(r.total)],
      ['Sangrias', moeda(rel.sangrias)],
      ['Suprimentos', moeda(rel.suprimentos)],
      ['Dinheiro esperado na gaveta', moeda(rel.dinheiro_esperado)],
    ];
    area.replaceChildren(
      el('p', { class: 'texto-grande' }, `Aberto em ${dataHora(s.sessao.aberto_em)} por ${s.sessao.aberto_por}.`),
      el('div', { class: 'indicadores-caixa' }, indicadores.map(([t, v]) =>
        el('div', { class: 'indicador' }, el('span', { class: 'rotulo' }, t), el('strong', {}, v)))),
      el('h4', {}, 'Vendas e ordens recebidas neste caixa'),
      tabela([
        { key: 'tipo_texto', label: 'Tipo' },
        { key: 'ordem', label: 'Número' },
        { key: 'cliente_texto', label: 'Cliente' },
        { key: 'total_texto', label: 'Total da venda' },
        { key: 'recebido_texto', label: 'Recebido neste caixa' },
        { key: 'forma', label: 'Forma de pagamento' },
      ], vendas, 'Nenhuma venda recebida neste caixa ainda.'));
  } catch (e) {
    area.replaceChildren(el('p', { class: 'ajuda', role: 'alert' }, e?.texto ?? 'Não foi possível carregar o caixa.'));
  }
}

async function situacaoCaixa() {
  try {
    return await chamar('caixa_situacao');
  } catch (_) {
    return null;
  }
}

function cartaoCaixa(s, ctx) {
  if (!s) return null;
  if (!s.aberto) {
    return el('div', { class: 'caixa-resumo fechado' },
      el('div', {},
        el('span', { class: 'rotulo-grande' }, 'Caixa fechado'),
        el('p', { class: 'texto-grande' }, 'Abra o caixa para receber os pagamentos do dia.')),
      el('button', { type: 'button', class: 'botao-grande primario', onclick: () => ctx.irPara('caixa') },
        icone('caixa', 30), el('span', {}, 'Abrir o caixa')));
  }
  const outro = Boolean(s.de_outro_operador);
  return el('div', { class: 'caixa-resumo aberto' },
    el('div', {},
      el('span', { class: 'rotulo-grande' }, outro ? 'Caixa aberto por outra pessoa' : 'Caixa aberto'),
      el('p', { class: 'texto-grande' }, outro
        ? `Aberto por ${s.sessao.aberto_por}. Só quem abriu ou o administrador pode fechá-lo.`
        : `Recebido até agora: ${moeda(s.recebimentos.total)} (${s.recebimentos.atendimentos} atendimento(s))`)),
    el('button', { type: 'button', class: 'botao-grande', onclick: () => ctx.irPara('caixa') },
      icone('caixa', 30), el('span', {}, outro ? 'Ver o caixa' : 'Ver ou fechar o caixa')));
}

function bloco(tela, ctx) {
  return el('button', {
    type: 'button', class: `bloco cor-${tela.cor}`, onclick: () => ctx.irPara(tela.id),
    'aria-label': `${tela.titulo}. Atalho ${tela.tecla}`,
  },
  el('span', { class: 'bloco-topo' },
    el('span', { class: 'bloco-icone' }, icone(tela.icone, 46)),
    el('kbd', { class: 'tecla' }, tela.tecla)),
  el('span', { class: 'bloco-titulo' }, tela.titulo),
  el('span', { class: 'bloco-desc' }, tela.descricao));
}

export async function render(container, ctx) {
  const situacao = await situacaoCaixa();
  const area = el('div', { class: 'caixas-abertos-area' });
  const telas = (ctx.telas ?? []).filter((t) => t.tile);
  container.replaceChildren(
    el('section', { class: 'inicio' },
      el('div', { class: 'pergunta' },
        el('h2', {}, 'O que você deseja fazer agora?'),
        el('p', { class: 'texto-grande' }, 'Toque em um dos blocos ou use a tecla de atalho indicada em cada um.')),
      cartaoCaixa(situacao, ctx),
      ctx.admin ? el('section', { class: 'card', 'aria-label': 'Caixas abertos' },
        el('h3', {}, 'Caixas abertos'),
        el('p', { class: 'ajuda' }, 'Mostra o caixa aberto agora, com todos os valores e as vendas recebidas nele. Só o administrador vê isto.'),
        botao({ texto: 'Ver caixas abertos', icone: 'caixa', tipo: 'primario', onclick: () => painelCaixasAbertos(area) }),
        area) : null,
      el('div', { class: 'grade-blocos' }, telas.map((t) => bloco(t, ctx)))),
  );
}
