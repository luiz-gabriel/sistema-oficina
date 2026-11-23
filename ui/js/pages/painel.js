// Painel do administrador: faturamento, recebimentos, contas, serviços e estoque, em gráficos e tabelas.
// Só leitura. A tela só aparece para o administrador, e o backend também recusa a consulta para funcionário.
// Os gráficos são desenhados em SVG por código (sem biblioteca). Cada um tem título, explicação e texto de apoio,
// e as cores vêm sempre junto com rótulos escritos, para não depender só de cor.
import { chamar } from '../api.js';
import { el, aviso, botao, cabecalhoPagina, moeda } from '../components.js';

const NS = 'http://www.w3.org/2000/svg';
const COR_PRINCIPAL = '#1f5fa8'; // azul (contraste alto sobre branco)
const COR_PAGAR = '#7a4a00';     // marrom escuro (contraste alto sobre branco)
const COR_VENCIDO = '#b42318';   // vermelho escuro (contraste alto sobre branco)
const COR_TEXTO = '#1d1d1f';
const COR_EIXO = '#5b5b5b';
const COR_GRADE = '#cfcfcf';

const FORMAS = { dinheiro: 'Dinheiro', pix: 'Pix', cartao: 'Cartão', outro: 'Outro' };
const SITUACOES = {
  CRIADA: 'Criada, ainda não começou',
  EM_ANDAMENTO: 'Em andamento',
  AGUARDANDO_PAGAMENTO: 'Serviço pronto, aguardando pagamento',
  CONCLUIDA: 'Concluída e paga',
  CANCELADA: 'Cancelada',
};

// Datas no fuso do computador, em AAAA-MM-DD (formato que o backend recebe).
const iso = (d) => `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
const br = (aaaaMmDd) => aaaaMmDd.split('-').reverse().join('/');

// Períodos rápidos. Nenhum deles vai além de hoje, para o gráfico não mostrar dias futuros sem venda.
//  - Últimos 7 dias: de 6 dias atrás até hoje (7 dias no total, com hoje).
//  - Este mês: do dia 1 do mês até hoje.
//  - Últimos 12 meses: do dia 1 do mês de 11 meses atrás até hoje (12 meses no total, com este).
const PERIODOS = [
  ['7dias', 'Últimos 7 dias'],
  ['mes', 'Este mês'],
  ['12meses', 'Últimos 12 meses'],
];

function periodo(tipo) {
  const hoje = new Date();
  if (tipo === '7dias') {
    const de = new Date(hoje);
    de.setDate(hoje.getDate() - 6);
    return { de, ate: hoje };
  }
  if (tipo === 'mes') return { de: new Date(hoje.getFullYear(), hoje.getMonth(), 1), ate: hoje };
  return { de: new Date(hoje.getFullYear(), hoje.getMonth() - 11, 1), ate: hoje };
}

// Valores em reais completos (ex.: R$ 1.230,00) e em reais inteiros para os números do eixo (ex.: R$ 300).
const reais = (c) => moeda((c ?? 0) / 100);
const reaisInteiros = (c) => `R$ ${Math.round((c ?? 0) / 100).toLocaleString('pt-BR')}`;
const plural = (n, um, varios) => `${n} ${n === 1 ? um : varios}`;

// Elemento SVG (o `el` comum cria só elementos HTML).
function s(tag, attrs = {}, ...filhos) {
  const no = document.createElementNS(NS, tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (v !== undefined && v !== null && v !== false) no.setAttribute(k, String(v));
  }
  for (const f of filhos.flat()) {
    if (f === null || f === undefined || f === false) continue;
    no.append(f instanceof Node ? f : document.createTextNode(String(f)));
  }
  return no;
}

// Caixa SVG com viewBox: escala para a largura da tela (cabe em celular sem barra de rolagem lateral).
function svgCaixa(largura, altura, descricao, nos) {
  return s('svg', {
    viewBox: `0 0 ${largura} ${altura}`, role: 'img', 'aria-label': descricao, class: 'grafico-svg',
    style: 'width:100%;height:auto;max-width:640px;display:block',
  }, s('title', {}, descricao), ...nos);
}

// Gráfico de barras verticais: faturamento por dia.
function graficoFaturamento(dias) {
  const W = 600, H = 300, E = 96, D = 16, T = 24, B = 258;
  const maximo = Math.max(0, ...dias.map((d) => d.total_centavos));
  const topo = maximo > 0 ? maximo : 100; // sem venda: eixo de R$ 0 a R$ 1, para não dividir por zero
  const n = dias.length;
  const largura = (W - E - D) / n;
  // Com muitos dias, mostra o rótulo de só alguns (no máximo cerca de dez), para caber.
  const passo = Math.ceil(n / 10);
  const nos = [];
  for (const f of [0, 0.5, 1]) {
    const y = B - f * (B - T);
    nos.push(s('line', { x1: E, x2: W - D, y1: y, y2: y, stroke: f === 0 ? COR_EIXO : COR_GRADE, 'stroke-width': f === 0 ? 2 : 1 }));
    nos.push(s('text', { x: E - 10, y: y + 6, 'text-anchor': 'end', 'font-size': 18, fill: COR_TEXTO }, reaisInteiros(topo * f)));
  }
  dias.forEach((d, i) => {
    const altura = (d.total_centavos / topo) * (B - T);
    const x = E + i * largura + largura * 0.15;
    const bw = largura * 0.7;
    nos.push(s('rect', { x, y: B - altura, width: bw, height: altura, fill: COR_PRINCIPAL }));
    if (i % passo === 0) {
      nos.push(s('text', { x: x + bw / 2, y: B + 26, 'text-anchor': 'middle', 'font-size': 18, fill: COR_TEXTO }, br(d.dia).slice(0, 5)));
    }
  });
  return svgCaixa(W, H, 'Gráfico de barras do faturamento por dia, em reais', nos);
}

// Barras horizontais: uma linha por item, com o nome em cima da barra e o valor depois dela.
function graficoBarrasHorizontais(linhas, descricao) {
  const W = 600, LINHA = 66, TOPO = 8, LARGURA_MAX = 400;
  const maximo = Math.max(1, ...linhas.map((l) => l.valor));
  const H = TOPO + linhas.length * LINHA + 6;
  const nos = linhas.flatMap((l, i) => {
    const y = TOPO + i * LINHA;
    const w = Math.max(0, (l.valor / maximo) * LARGURA_MAX);
    return [
      s('text', { x: 0, y: y + 22, 'font-size': 22, fill: COR_TEXTO }, l.rotulo),
      s('rect', { x: 0, y: y + 30, width: w, height: 28, fill: COR_PRINCIPAL }),
      s('text', { x: w + 10, y: y + 53, 'font-size': 22, fill: COR_TEXTO }, l.texto),
    ];
  });
  return svgCaixa(W, H, descricao, nos);
}

// Barras verticais com rótulo em duas linhas embaixo e valor em cima de cada barra.
function graficoBarrasVerticais(itens, descricao) {
  const W = 600, H = 300, T = 48, B = 232, LARGURA_BARRA = 96;
  const maximo = Math.max(1, ...itens.map((i) => i.valor));
  const slot = W / itens.length;
  const nos = itens.flatMap((item, i) => {
    const altura = (item.valor / maximo) * (B - T);
    const x = i * slot + (slot - LARGURA_BARRA) / 2;
    return [
      s('rect', { x, y: B - altura, width: LARGURA_BARRA, height: altura, fill: item.cor }),
      s('text', { x: x + LARGURA_BARRA / 2, y: B - altura - 10, 'text-anchor': 'middle', 'font-size': 20, fill: COR_TEXTO }, reais(item.valor)),
      s('text', { x: x + LARGURA_BARRA / 2, y: B + 30, 'text-anchor': 'middle', 'font-size': 20, fill: COR_TEXTO }, item.rotulo[0]),
      s('text', { x: x + LARGURA_BARRA / 2, y: B + 56, 'text-anchor': 'middle', 'font-size': 20, fill: COR_TEXTO }, item.rotulo[1]),
    ];
  });
  nos.unshift(s('line', { x1: 0, x2: W, y1: B, y2: B, stroke: COR_EIXO, 'stroke-width': 2 }));
  return svgCaixa(W, 300, descricao, nos);
}

function tabelaSimples(colunas, linhas) {
  return el('div', { class: 'tabela-wrap' }, el('table', {},
    el('thead', {}, el('tr', {}, colunas.map((c) => el('th', {}, c)))),
    el('tbody', {}, linhas.map((l) => el('tr', {}, l.map((v) => el('td', {}, String(v))))))));
}

function secao(titulo, explicacao, ...conteudo) {
  return el('section', { class: 'card secao painel-grafico' },
    el('h3', {}, titulo),
    el('p', { class: 'ajuda' }, explicacao),
    ...conteudo);
}

function montarPainel(d) {
  const totalFat = d.faturamento_por_dia.reduce((soma, x) => soma + x.total_centavos, 0);
  const qtdFat = d.faturamento_por_dia.reduce((soma, x) => soma + x.quantidade, 0);
  const c = d.contas;
  const servicos = d.servicos_mais_realizados;
  const baixo = d.estoque_baixo;

  return [
    secao('Faturamento por dia',
      'Mostra quanto a oficina faturou em cada dia. Entram as ordens concluídas e as que aguardam pagamento. Dia sem barra é dia sem venda.',
      el('p', { class: 'texto-grande' }, qtdFat
        ? `Total no período: ${reais(totalFat)}, em ${plural(qtdFat, 'ordem ou venda', 'ordens e vendas')}.`
        : 'Nenhuma venda concluída neste período.'),
      graficoFaturamento(d.faturamento_por_dia)),

    secao('Recebimentos por forma de pagamento',
      'Quanto entrou em cada forma de pagamento no período. Os valores estão escritos ao lado de cada barra.',
      graficoBarrasHorizontais(
        d.recebimentos_por_forma.map((f) => ({ rotulo: FORMAS[f.forma] ?? f.forma, valor: f.total_centavos, texto: reais(f.total_centavos) })),
        'Barras horizontais dos recebimentos por forma de pagamento, em reais')),

    secao('Contas a receber e a pagar (situação de hoje)',
      '"Total" é tudo o que ainda falta. "Vencido" é a parte cujo prazo já passou. Este gráfico não depende do período escolhido.',
      graficoBarrasVerticais([
        { rotulo: ['A receber', 'total'], valor: c.receber_aberto, cor: COR_PRINCIPAL },
        { rotulo: ['A receber', 'vencido'], valor: c.receber_vencido, cor: COR_VENCIDO },
        { rotulo: ['A pagar', 'total'], valor: c.pagar_aberto, cor: COR_PAGAR },
        { rotulo: ['A pagar', 'vencido'], valor: c.pagar_vencido, cor: COR_VENCIDO },
      ], 'Barras de contas a receber e a pagar, total e vencido, em reais')),

    secao('Serviços mais realizados',
      'Os cinco serviços feitos mais vezes no período, do mais feito ao menos feito.',
      servicos.length
        ? graficoBarrasHorizontais(
          servicos.map((x) => ({ rotulo: x.nome, valor: x.quantidade, texto: plural(x.quantidade, 'vez', 'vezes') })),
          'Barras horizontais dos cinco serviços mais realizados, com a quantidade de vezes')
        : el('p', { class: 'vazio' }, 'Nenhum serviço concluído neste período.')),

    secao('Peças com estoque baixo (agora)',
      'Peças com quantidade igual ou menor que o mínimo cadastrado. Vale comprar antes que acabe.',
      baixo.length
        ? tabelaSimples(['Peça', 'Quantidade em estoque', 'Mínimo'],
          baixo.map((x) => [x.nome, x.quantidade, x.estoque_minimo]))
        : el('p', { class: 'vazio' }, 'Nenhuma peça com estoque baixo.')),

    secao('Ordens de serviço por situação',
      'Quantas ordens abertas no período estão em cada situação hoje. Ordem cancelada também aparece aqui.',
      tabelaSimples(['Situação', 'Quantidade'],
        d.os_por_situacao.map((x) => [SITUACOES[x.situacao] ?? x.situacao, x.quantidade]))),
  ];
}

export async function render(container, ctx) {
  let atual = '7dias';
  let geracao = 0; // ignora respostas antigas quando o administrador troca de período depressa
  const area = el('div', { class: 'painel-area', 'aria-live': 'polite' });
  const info = el('p', { class: 'ajuda' });
  const botoes = new Map();

  function marcarBotoes() {
    for (const [tipo, b] of botoes) {
      const ativo = tipo === atual;
      b.className = `btn ${ativo ? 'primario' : 'secundario'}`;
      b.setAttribute('aria-pressed', ativo ? 'true' : 'false');
    }
  }

  async function carregar() {
    const seq = ++geracao;
    const p = periodo(atual);
    const de = iso(p.de);
    const ate = iso(p.ate);
    info.textContent = `Período: de ${br(de)} a ${br(ate)}.`;
    area.replaceChildren(el('p', { class: 'ajuda' }, 'Carregando o painel...'));
    try {
      const dados = await chamar('painel_serie', { de, ate });
      if (seq !== geracao) return;
      area.replaceChildren(...montarPainel(dados));
    } catch (e) {
      if (seq !== geracao) return;
      area.replaceChildren();
      aviso(e.texto ?? 'Não foi possível carregar o painel.', 'erro');
    }
  }

  for (const [tipo, texto] of PERIODOS) {
    botoes.set(tipo, botao({
      texto,
      tipo: 'secundario',
      onclick: () => {
        if (atual === tipo) return;
        atual = tipo;
        marcarBotoes();
        carregar();
      },
    }));
  }
  marcarBotoes();

  container.replaceChildren(
    cabecalhoPagina('Painel da administração', 'registro',
      'Resumo da oficina em gráficos. Só o administrador vê esta tela. Ela só mostra dados, não muda nada.'),
    el('section', { class: 'card' },
      el('h3', {}, 'Escolha o período'),
      el('div', { class: 'acoes' }, [...botoes.values()]),
      el('p', { class: 'ajuda' },
        'Últimos 7 dias: de 6 dias atrás até hoje. Este mês: do dia 1 até hoje. Últimos 12 meses: do dia 1 de 11 meses atrás até hoje.'),
      info),
    area);
  carregar();
}
