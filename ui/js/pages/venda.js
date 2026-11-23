// Venda rápida de balcão: peças e serviços vendidos sem carro, com cliente opcional e pagamento na hora.
// Concluir exige caixa aberto (o servidor confere de novo, então a tela só antecipa o aviso).
import { chamar } from '../api.js';
import { el, aviso, botao, campo, icone, moeda, cabecalhoPagina } from '../components.js';

const PAGAMENTOS = [['dinheiro', 'Dinheiro'], ['pix', 'Pix'], ['cartao', 'Cartão'], ['outro', 'Outro']];
const ROTULO_TIPO = { peca: 'Peça', servico: 'Serviço' };
const MAX_RESULTADOS = 8;

// Texto de erro do servidor: em validação, junta as mensagens dos campos.
function textoErro(e) {
  if (e?.campos && Object.keys(e.campos).length) return Object.values(e.campos).join(' ');
  return e?.texto ?? String(e);
}

export async function render(container, ctx = {}) {
  const [itens, clientes, funcionarios, situacao] = await Promise.all([
    chamar('itens_listar'),
    chamar('clientes_listar'),
    chamar('funcionarios_listar'),
    chamar('caixa_situacao'),
  ]);

  // Quem está logado vira o atendente padrão (o campo continua editável).
  const login = ctx.estado?.()?.logado?.usuario;
  const atendentePadrao = funcionarios.find((f) => f.login && f.login === login)?.id ?? '';

  const carrinho = new Map(); // item_id -> { item, quantidade }
  let cliente = null; // null = venda avulsa, sem cliente identificado

  // ----- Caixa -----
  let caixaAberto = situacao.aberto;
  const caixaArea = el('div', {});
  function desenharCaixa() {
    caixaArea.replaceChildren(caixaAberto ? null : el('div', { class: 'faixa alerta', role: 'alert' },
      icone('alerta', 26),
      el('p', {}, 'O caixa está fechado. Abra o caixa para concluir a venda.'),
      botao({ texto: 'Abrir o caixa', icone: 'caixa', tipo: 'primario', onclick: () => ctx.irPara?.('caixa') })));
    desenharBotao();
  }

  // ----- Busca de peça ou serviço -----
  const buscaItem = el('input', { type: 'search', placeholder: 'Nome ou código de barras', 'aria-label': 'Buscar peça ou serviço', autocomplete: 'off' });
  const resultadosItem = el('div', { class: 'resultados-busca' });

  function filtrarItens(texto) {
    const t = texto.trim().toLowerCase();
    if (!t) return [];
    return itens
      .filter((i) => i.nome.toLowerCase().includes(t) || (i.codigo ?? '').toLowerCase().includes(t))
      .slice(0, MAX_RESULTADOS);
  }

  function adicionar(item) {
    const atual = carrinho.get(item.id);
    if (atual) atual.quantidade += 1;
    else carrinho.set(item.id, { item, quantidade: 1 });
    buscaItem.value = '';
    resultadosItem.replaceChildren();
    desenharCarrinho();
    buscaItem.focus();
  }

  buscaItem.addEventListener('input', () => {
    resultadosItem.replaceChildren(...filtrarItens(buscaItem.value).map((i) => el('div', { class: 'linha-resultado' },
      el('span', {}, `${i.nome} · ${ROTULO_TIPO[i.tipo] ?? i.tipo} · ${moeda(i.preco_venda)}`),
      i.tipo === 'peca' ? el('small', { class: 'ajuda' }, ` estoque ${i.quantidade}`) : null,
      botao({ texto: 'Adicionar', icone: 'mais', tipo: 'primario', onclick: () => adicionar(i) }))));
  });

  // Enter com código de barras: adiciona direto quando o código bate com um item.
  buscaItem.addEventListener('keydown', async (ev) => {
    if (ev.key !== 'Enter') return;
    ev.preventDefault();
    const codigo = buscaItem.value.trim();
    if (!codigo) return;
    const local = itens.find((i) => (i.codigo ?? '').toLowerCase() === codigo.toLowerCase());
    if (local) return adicionar(local);
    try {
      adicionar(await chamar('itens_buscar_codigo', { codigo }));
    } catch (e) {
      aviso(textoErro(e), 'erro');
    }
  });

  // ----- Itens da venda -----
  const carrinhoArea = el('div', { class: 'card' });
  function desenharCarrinho() {
    const linhas = [...carrinho.values()];
    const total = linhas.reduce((soma, l) => soma + l.item.preco_venda * l.quantidade, 0);
    carrinhoArea.replaceChildren(
      el('h3', {}, icone('ordens', 24), 'Itens da venda'),
      linhas.length
        ? el('div', { class: 'itens-os' }, linhas.map((l) => el('div', { class: 'linha-item' },
          el('span', { class: 'rotulo' }, `${l.item.nome} (${moeda(l.item.preco_venda)} cada)`),
          el('label', { class: 'campo' },
            el('span', { class: 'rotulo' }, 'Quantidade'),
            el('input', {
              type: 'number', min: 1, step: 1, value: l.quantidade, 'aria-label': `Quantidade de ${l.item.nome}`,
              onchange: (ev) => {
                l.quantidade = Math.max(1, Math.floor(Number(ev.target.value) || 1));
                desenharCarrinho();
              },
            })),
          l.item.tipo === 'peca' && l.quantidade > l.item.quantidade
            ? el('small', { class: 'erro-campo' }, `Estoque atual: ${l.item.quantidade}. Falta peça para esta venda.`)
            : null,
          el('strong', {}, moeda(l.item.preco_venda * l.quantidade)),
          el('button', { type: 'button', class: 'link', onclick: () => { carrinho.delete(l.item.id); desenharCarrinho(); } }, 'remover'))))
        : el('p', { class: 'vazio' }, 'Nenhum item ainda. Busque uma peça ou serviço acima.'),
      el('p', { class: 'total-venda' }, el('span', {}, 'Total da venda: '), el('strong', {}, moeda(total))),
    );
    desenharBotao();
  }

  // ----- Cliente (opcional) -----
  const clienteArea = el('div', { class: 'card' });
  const buscaCliente = el('input', { type: 'search', placeholder: 'Nome ou telefone', 'aria-label': 'Buscar cliente', autocomplete: 'off' });
  const resultadosCliente = el('div', { class: 'resultados-busca' });

  function filtrarClientes(texto) {
    const t = texto.trim().toLowerCase();
    if (!t) return [];
    return clientes
      .filter((c) => c.nome.toLowerCase().includes(t) || (c.telefone ?? '').toLowerCase().includes(t))
      .slice(0, MAX_RESULTADOS);
  }

  buscaCliente.addEventListener('input', () => {
    resultadosCliente.replaceChildren(...filtrarClientes(buscaCliente.value).map((c) => el('div', { class: 'linha-resultado' },
      el('span', {}, c.nome), c.telefone ? el('small', { class: 'ajuda' }, ` ${c.telefone}`) : null,
      botao({ texto: 'Escolher', icone: 'ok', onclick: () => { cliente = c; desenharCliente(); } }))));
  });

  function desenharCliente() {
    if (cliente) {
      clienteArea.replaceChildren(
        el('h3', {}, icone('pessoa', 24), 'Cliente'),
        el('p', {}, `Venda em nome de ${cliente.nome}. Os pontos de fidelidade vão para este cliente.`),
        botao({ texto: 'Tirar o cliente desta venda', icone: 'x', onclick: () => { cliente = null; buscaCliente.value = ''; desenharCliente(); } }),
      );
      return;
    }
    buscaCliente.value = '';
    resultadosCliente.replaceChildren();
    clienteArea.replaceChildren(
      el('h3', {}, icone('pessoa', 24), 'Cliente (opcional)'),
      el('p', { class: 'ajuda' }, 'Sem cliente, a venda fica avulsa e não gera pontos de fidelidade.'),
      el('label', { class: 'campo' }, el('span', { class: 'rotulo' }, 'Buscar cliente'), buscaCliente),
      resultadosCliente,
    );
  }

  // ----- Pagamento e atendente -----
  const campoPagamento = campo({ name: 'pagamento', label: 'Forma de pagamento *', type: 'select', required: true, options: PAGAMENTOS }, '');
  const selPagamento = campoPagamento.querySelector('select');
  selPagamento.addEventListener('change', desenharBotao);

  const campoAtendente = campo({
    name: 'atendente', label: 'Atendente *', type: 'select', required: true,
    options: funcionarios.map((f) => [f.id, f.nome]),
  }, atendentePadrao);
  const selAtendente = campoAtendente.querySelector('select');
  selAtendente.addEventListener('change', desenharBotao);

  // ----- Concluir -----
  const botaoConcluir = botao({ texto: 'Concluir venda', icone: 'ok', tipo: 'primario', onclick: concluir });
  const mensagemArea = el('p', { class: 'ajuda', role: 'status' }, '');

  function desenharBotao() {
    const faltaAlgo = carrinho.size === 0 || !selPagamento.value || !selAtendente.value;
    botaoConcluir.disabled = !caixaAberto || faltaAlgo;
    mensagemArea.textContent = !caixaAberto
      ? 'Abra o caixa para concluir a venda.'
      : carrinho.size === 0 ? 'Adicione pelo menos um item.' : !selPagamento.value ? 'Escolha a forma de pagamento.' : '';
  }

  async function concluir() {
    const dados = {
      cliente_id: cliente?.id ?? null,
      funcionario_id: Number(selAtendente.value),
      pagamento: selPagamento.value,
      itens: [...carrinho.values()].map((l) => ({ item_id: l.item.id, quantidade: l.quantidade })),
    };
    botaoConcluir.disabled = true;
    try {
      const venda = await chamar('venda_balcao_criar', { dados });
      const forma = PAGAMENTOS.find(([v]) => v === venda.pagamento)?.[1] ?? venda.pagamento;
      aviso(`Venda concluída: ${moeda(venda.valor_total)} recebidos em ${forma}.`);
      // Estoque e caixa mudaram: recarrega a tela com a venda limpa.
      await render(container, ctx);
    } catch (e) {
      const texto = textoErro(e);
      if (/caixa/i.test(texto)) {
        caixaAberto = false;
        desenharCaixa();
      }
      aviso(texto, 'erro');
      desenharBotao();
    }
  }

  // ----- Montagem -----
  desenharCaixa();
  desenharCliente();
  desenharCarrinho();
  container.replaceChildren(
    cabecalhoPagina('Venda Rápida', 'caixa', 'Venda de peças e serviços sem carro. Cliente é opcional.'),
    caixaArea,
    el('div', { class: 'card' },
      el('h3', {}, icone('pecas', 24), 'Buscar peça ou serviço'),
      el('label', { class: 'campo' }, el('span', { class: 'rotulo' }, 'Nome ou código'), buscaItem),
      resultadosItem),
    carrinhoArea,
    clienteArea,
    el('div', { class: 'card' },
      el('h3', {}, icone('caixa', 24), 'Pagamento'),
      campoPagamento,
      campoAtendente),
    el('div', { class: 'acoes' }, botaoConcluir),
    mensagemArea,
  );
  desenharBotao();
  buscaItem.focus();
}
