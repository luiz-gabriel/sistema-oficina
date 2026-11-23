// Compras e fornecedores: cadastro de fornecedores (CPF/CNPJ), registro de compra (entrada de estoque e custo médio)
// e histórico. A tela não aparece no menu até que app.js a registre (ver a linha informada no relatório).
import { chamar } from '../api.js';
import { el, aviso, botao, icone, cabecalhoPagina, campo, tabela, moeda } from '../components.js';

const FORMAS = [
  ['pix', 'PIX'], ['dinheiro', 'Dinheiro'], ['cartao', 'Cartão'],
  ['boleto', 'Boleto'], ['transferencia', 'Transferência'], ['a pagar', 'A pagar (a prazo)'],
];
const A_PAGAR = 'a pagar';

const semAcento = (t) => String(t ?? '').normalize('NFD').replace(/[̀-ͯ]/g, '').toLowerCase();
const hojeIso = () => new Date().toLocaleDateString('sv-SE'); // formato AAAA-MM-DD, no fuso local
const dataBr = (iso) => (iso ? iso.split('-').reverse().join('/') : '—');
const formaLegivel = (valor) => FORMAS.find(([v]) => v === valor)?.[1] ?? valor;
const situacao = (c) => (c.paga ? 'paga' : `a pagar (vence ${dataBr(c.vencimento)})`);

function formatarDocumento(d) {
  const n = String(d ?? '').replace(/\D/g, '');
  if (n.length === 11) return n.replace(/^(\d{3})(\d{3})(\d{3})(\d{2})$/, '$1.$2.$3-$4');
  if (n.length === 14) return n.replace(/^(\d{2})(\d{3})(\d{3})(\d{4})(\d{2})$/, '$1.$2.$3/$4-$5');
  return String(d ?? '');
}

// "12,50", "12.50" ou "R$ 1.234,50" viram centavos inteiros. Retorna null se não for um valor válido.
function paraCentavos(texto) {
  let s = String(texto ?? '').trim().replace(/^R\$\s*/, '');
  if (s.includes(',')) s = s.replace(/\./g, '').replace(',', '.');
  if (!/^\d+(\.\d{1,2})?$/.test(s)) return null;
  return Math.round(Number(s) * 100);
}

// Rótulo grande ao redor de um controle (mesmo visual dos campos de formulário).
const rotulado = (texto, controle, extra = {}) =>
  el('label', { class: 'campo', ...extra }, el('span', { class: 'rotulo' }, texto), controle);

// Erros de validação chegam com uma mensagem por campo; mostra todas juntas.
function mensagemDeErro(e) {
  const msgs = e?.campos ? Object.values(e.campos) : [];
  return msgs.length ? msgs.join(' ') : (e?.texto ?? 'Não foi possível concluir a operação.');
}

function blocoCompra(c) {
  return el('div', { class: 'bloco-compra' },
    el('p', {}, el('strong', {}, `${dataBr(c.data)} · ${c.fornecedor_nome}`),
      ` · ${moeda(c.total_centavos / 100)} · ${formaLegivel(c.pagamento)} · ${situacao(c)}`),
    tabela([
      { label: 'Peça', key: 'item_nome' },
      { label: 'Quantidade', key: 'quantidade' },
      { label: 'Custo unitário', render: (l) => moeda(l.custo_unitario_centavos / 100) },
      { label: 'Subtotal', render: (l) => moeda(l.subtotal_centavos / 100) },
    ], c.itens, 'Compra sem peças.'));
}

export async function render(container) {
  const st = { fornecedores: [], pecas: [], compras: [], linhas: [], fornecedorEditando: null };

  // Áreas fixas: ficam na tela e são atualizadas com o conteúdo novo.
  const areaRegistro = el('section', { class: 'card' });
  const areaFornecedor = el('section', { class: 'card' });
  const areaCompras = el('section', { class: 'card' });
  const areaHistorico = el('section', { class: 'card', hidden: '' });

  const buscaForn = el('input', { type: 'search', placeholder: 'Buscar por nome, CPF ou CNPJ', autocomplete: 'off' });
  const formForn = el('div');
  const listaForn = el('div');
  const listaCompras = el('div');
  const selPeca = el('select', { 'aria-label': 'Peça para ver as entradas' });

  container.replaceChildren(
    cabecalhoPagina('Compras e Fornecedores', 'pecas',
      'Registre o que chegou do fornecedor: a peça entra no estoque e o custo médio é atualizado.'),
    areaRegistro, areaFornecedor, areaCompras, areaHistorico,
  );

  function mostrarArea(titulo, conteudo) {
    areaHistorico.hidden = false;
    areaHistorico.replaceChildren(
      el('h3', {}, icone('seta', 22), titulo),
      conteudo,
      botao({ texto: 'Fechar', icone: 'x', onclick: () => { areaHistorico.hidden = true; } }),
    );
    areaHistorico.scrollIntoView({ behavior: 'smooth', block: 'start' });
  }

  // ---- Registro de compra ----
  function desenharRegistro() {
    const fornecedorSel = el('select', { name: 'fornecedor_id' },
      el('option', { value: '' }, '— escolha o fornecedor —'),
      st.fornecedores.map((f) => el('option', { value: f.id }, `${f.nome} · ${formatarDocumento(f.documento)}`)));
    const dataInput = el('input', { type: 'date', value: hojeIso() });
    const pagSel = el('select', { name: 'pagamento' },
      el('option', { value: '' }, '— escolha —'),
      FORMAS.map(([v, t]) => el('option', { value: v }, t)));
    const vencInput = el('input', { type: 'date' });
    const vencLabel = rotulado('Vencimento da conta a pagar *', vencInput, { hidden: '' });
    pagSel.addEventListener('change', () => { vencLabel.hidden = pagSel.value !== A_PAGAR; });

    const buscaInput = el('input', { type: 'search', placeholder: 'Digite parte do nome da peça', autocomplete: 'off' });
    const sugestoes = el('div', { class: 'sugestoes' });
    const escolhida = el('p', { class: 'ajuda' }, 'Nenhuma peça escolhida.');
    const qtdInput = el('input', { type: 'number', min: '1', step: '1', inputmode: 'numeric' });
    const custoInput = el('input', { type: 'text', inputmode: 'decimal', placeholder: 'ex.: 12,50' });
    const areaLinhas = el('div');
    const totalEl = el('p', { class: 'total' });
    const erroArea = el('div', { role: 'alert' });
    let pecaEscolhida = null;

    function escolher(p) {
      pecaEscolhida = p;
      buscaInput.value = p.nome;
      sugestoes.replaceChildren();
      escolhida.textContent = `Peça escolhida: ${p.nome} (estoque atual ${p.quantidade})`;
      qtdInput.focus();
    }

    buscaInput.addEventListener('input', () => {
      pecaEscolhida = null;
      escolhida.textContent = 'Nenhuma peça escolhida.';
      const t = semAcento(buscaInput.value).trim();
      const achadas = t ? st.pecas.filter((p) => semAcento(p.nome).includes(t)).slice(0, 8) : [];
      sugestoes.replaceChildren(...achadas.map((p) => botao({ texto: `${p.nome} (estoque ${p.quantidade})`, onclick: () => escolher(p) })));
    });

    function desenharLinhas() {
      const total = st.linhas.reduce((soma, l) => soma + l.quantidade * l.custo, 0);
      const linhas = st.linhas.map((l, i) => ({
        ...l,
        subtotal: l.quantidade * l.custo,
        __acoes: botao({ texto: 'Remover', icone: 'lixeira', tipo: 'link perigo', onclick: () => { st.linhas.splice(i, 1); desenharLinhas(); } }),
      }));
      areaLinhas.replaceChildren(tabela([
        { label: 'Peça', key: 'nome' },
        { label: 'Quantidade', key: 'quantidade' },
        { label: 'Custo unitário', render: (l) => moeda(l.custo / 100) },
        { label: 'Subtotal', render: (l) => moeda(l.subtotal / 100) },
      ], linhas, 'Nenhuma peça adicionada ainda.'));
      totalEl.replaceChildren('Total da compra: ', el('strong', {}, moeda(total / 100)));
    }

    function adicionarPeca() {
      if (!pecaEscolhida) return aviso('Escolha a peça na lista de busca.', 'erro');
      const quantidade = Number(qtdInput.value);
      if (!Number.isInteger(quantidade) || quantidade <= 0) return aviso('A quantidade deve ser um número maior que zero.', 'erro');
      const custo = paraCentavos(custoInput.value);
      if (custo === null) return aviso('Custo inválido. Use reais, como 12,50.', 'erro');
      st.linhas.push({ item_id: pecaEscolhida.id, nome: pecaEscolhida.nome, quantidade, custo });
      buscaInput.value = '';
      pecaEscolhida = null;
      escolhida.textContent = 'Nenhuma peça escolhida.';
      qtdInput.value = '';
      custoInput.value = '';
      desenharLinhas();
      buscaInput.focus();
    }

    async function registrar() {
      erroArea.replaceChildren();
      if (!st.linhas.length) return aviso('Adicione pelo menos uma peça à compra.', 'erro');
      const dados = {
        fornecedor_id: Number(fornecedorSel.value) || null,
        data: dataInput.value || null,
        pagamento: pagSel.value || null,
        vencimento: pagSel.value === A_PAGAR ? (vencInput.value || null) : null,
        itens: st.linhas.map((l) => ({ item_id: l.item_id, quantidade: l.quantidade, custo_unitario_centavos: l.custo })),
      };
      try {
        const compra = await chamar('compras_registrar', { dados });
        st.linhas = [];
        aviso(`Compra registrada: ${moeda(compra.total_centavos / 100)}. O estoque já foi atualizado.`);
        await carregar();
      } catch (e) {
        const texto = mensagemDeErro(e);
        erroArea.replaceChildren(el('p', { class: 'erro-campo' }, texto));
        aviso(texto, 'erro');
      }
    }

    const botaoRegistrar = botao({ texto: 'Registrar compra', icone: 'ok', tipo: 'primario', onclick: registrar });
    botaoRegistrar.style.cssText = 'width:100%;font-size:1.3rem;padding:20px;margin-top:12px';

    areaRegistro.replaceChildren(
      el('h3', {}, icone('mais', 24), 'Registrar compra'),
      rotulado('Fornecedor *', fornecedorSel),
      el('div', {},
        rotulado('Data da compra *', dataInput),
        rotulado('Forma de pagamento *', pagSel),
        vencLabel),
      el('h4', {}, 'Peças recebidas'),
      rotulado('Buscar peça pelo nome', buscaInput),
      sugestoes,
      escolhida,
      el('div', {},
        rotulado('Quantidade *', qtdInput),
        rotulado('Custo unitário pago (R$) *', custoInput)),
      botao({ texto: 'Adicionar peça', icone: 'mais', tipo: 'secundario', onclick: adicionarPeca }),
      areaLinhas,
      totalEl,
      erroArea,
      botaoRegistrar,
    );
    desenharLinhas();
  }

  // ---- Fornecedores ----
  const CAMPOS_FORNECEDOR = [
    { name: 'nome', label: 'Nome *', required: true },
    { name: 'documento', label: 'CPF ou CNPJ *', required: true, ajuda: 'Com ou sem pontuação. Os dígitos verificadores são conferidos.' },
    { name: 'telefone', label: 'Telefone' },
    { name: 'email', label: 'E-mail', type: 'email' },
    { name: 'endereco', label: 'Endereço' },
    { name: 'observacoes', label: 'Observações', type: 'textarea' },
  ];

  function desenharFormFornecedor(valores, erros = {}) {
    const form = el('form', { onsubmit: salvarFornecedor },
      el('h3', {}, icone(st.fornecedorEditando ? 'lapis' : 'mais', 22),
        st.fornecedorEditando ? 'Alterar fornecedor' : 'Novo fornecedor'),
      CAMPOS_FORNECEDOR.map((c) => campo(c, valores[c.name] ?? '', erros[c.name])),
      el('div', { class: 'acoes' },
        botao({ texto: st.fornecedorEditando ? 'Salvar alterações' : 'Cadastrar fornecedor', icone: 'ok', tipo: 'primario', type: 'submit' }),
        botao({ texto: 'Cancelar', icone: 'x', onclick: () => { st.fornecedorEditando = null; formForn.replaceChildren(); } })));
    formForn.replaceChildren(form);
  }

  function abrirFormFornecedor(f = null) {
    st.fornecedorEditando = f;
    const valores = f ? { ...f, documento: formatarDocumento(f.documento) } : {};
    desenharFormFornecedor(valores);
    formForn.scrollIntoView({ behavior: 'smooth', block: 'start' });
  }

  async function salvarFornecedor(ev) {
    ev.preventDefault();
    const bruto = Object.fromEntries(new FormData(ev.target).entries());
    const dados = {};
    for (const c of CAMPOS_FORNECEDOR) dados[c.name] = String(bruto[c.name] ?? '').trim() || null;
    const editando = st.fornecedorEditando;
    try {
      if (editando) await chamar('fornecedores_atualizar', { id: editando.id, dados });
      else await chamar('fornecedores_criar', { dados });
      aviso(editando ? 'Fornecedor atualizado.' : 'Fornecedor cadastrado.');
      st.fornecedorEditando = null;
      formForn.replaceChildren();
      await carregar();
    } catch (e) {
      desenharFormFornecedor(bruto, e.campos ?? {});
      aviso(mensagemDeErro(e), 'erro');
    }
  }

  async function verComprasDoFornecedor(f) {
    try {
      const lista = await chamar('compras_historico_fornecedor', { fornecedor_id: f.id });
      mostrarArea(`Compras de ${f.nome} (${formatarDocumento(f.documento)})`,
        lista.length ? lista.map(blocoCompra) : el('p', { class: 'vazio' }, 'Nenhuma compra deste fornecedor ainda.'));
    } catch (e) {
      aviso(mensagemDeErro(e), 'erro');
    }
  }

  async function listarFornecedores(termo = '') {
    try {
      const lista = termo.trim() ? await chamar('fornecedores_buscar', { termo }) : st.fornecedores;
      listaForn.replaceChildren(tabela([
        { label: 'Nome', key: 'nome' },
        { label: 'CPF/CNPJ', render: (f) => formatarDocumento(f.documento) },
        { label: 'Telefone', key: 'telefone' },
      ], lista.map((f) => ({
        ...f,
        __acoes: el('span', { class: 'botoes-linha' },
          botao({ texto: 'Editar', icone: 'lapis', tipo: 'link', onclick: () => abrirFormFornecedor(f) }),
          botao({ texto: 'Ver compras', icone: 'seta', tipo: 'link', onclick: () => verComprasDoFornecedor(f) })),
      })), 'Nenhum fornecedor encontrado.'));
    } catch (e) {
      aviso(mensagemDeErro(e), 'erro');
    }
  }

  let timerBusca = null;
  buscaForn.addEventListener('input', () => {
    clearTimeout(timerBusca);
    timerBusca = setTimeout(() => listarFornecedores(buscaForn.value), 250);
  });

  areaFornecedor.replaceChildren(
    el('h3', {}, icone('equipe', 24), 'Fornecedores'),
    el('div', { class: 'barra-acoes' },
      buscaForn,
      botao({ texto: 'Novo fornecedor', icone: 'mais', tipo: 'primario', onclick: () => abrirFormFornecedor(null) })),
    formForn,
    listaForn,
  );

  // ---- Compras registradas e histórico por peça ----
  function desenharCompras() {
    listaCompras.replaceChildren(tabela([
      { label: 'Data', render: (c) => dataBr(c.data) },
      { label: 'Fornecedor', key: 'fornecedor_nome' },
      { label: 'Total', render: (c) => moeda(c.total_centavos / 100) },
      { label: 'Pagamento', render: (c) => `${formaLegivel(c.pagamento)} · ${situacao(c)}` },
    ], st.compras.slice(0, 50).map((c) => ({
      ...c,
      __acoes: botao({ texto: 'Ver peças', icone: 'pecas', tipo: 'link', onclick: () => mostrarArea(`Compra de ${dataBr(c.data)}`, blocoCompra(c)) }),
    })), 'Nenhuma compra registrada ainda.'));

    const atual = selPeca.value;
    selPeca.replaceChildren(
      el('option', { value: '' }, '— escolha a peça —'),
      st.pecas.map((p) => el('option', { value: p.id }, p.nome)),
    );
    selPeca.value = atual;
  }

  async function verEntradasDaPeca() {
    const id = Number(selPeca.value);
    if (!id) return aviso('Escolha a peça.', 'erro');
    try {
      const entradas = await chamar('compras_historico_item', { item_id: id });
      const nome = st.pecas.find((p) => p.id === id)?.nome ?? 'peça';
      mostrarArea(`Entradas de ${nome}`, tabela([
        { label: 'Data', render: (e) => dataBr(e.data) },
        { label: 'Fornecedor', key: 'fornecedor_nome' },
        { label: 'Quantidade', key: 'quantidade' },
        { label: 'Custo unitário', render: (e) => moeda(e.custo_unitario_centavos / 100) },
        { label: 'Subtotal', render: (e) => moeda(e.subtotal_centavos / 100) },
      ], entradas, 'Esta peça ainda não teve entradas por compra.'));
    } catch (e) {
      aviso(mensagemDeErro(e), 'erro');
    }
  }

  areaCompras.replaceChildren(
    el('h3', {}, icone('caixa', 24), 'Compras registradas'),
    listaCompras,
    el('h4', {}, 'Histórico de uma peça'),
    el('div', { class: 'barra-acoes' },
      selPeca,
      botao({ texto: 'Ver entradas', icone: 'seta', tipo: 'secundario', onclick: verEntradasDaPeca })),
  );

  async function carregar() {
    try {
      const [fornecedores, itens, compras] = await Promise.all([
        chamar('fornecedores_listar'), chamar('itens_listar'), chamar('compras_listar'),
      ]);
      st.fornecedores = fornecedores;
      st.pecas = itens.filter((i) => i.tipo === 'peca');
      st.compras = compras;
      desenharRegistro();
      desenharCompras();
      await listarFornecedores(buscaForn.value);
    } catch (e) {
      aviso(mensagemDeErro(e), 'erro');
    }
  }

  await carregar();
}
