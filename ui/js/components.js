// Proteção global: null, undefined e false nunca viram o texto "null"/"undefined" na tela.
const _sem_vazios = (nos) => nos.flat().filter((n) => n !== null && n !== undefined && n !== false);
const _substituir = Element.prototype.replaceChildren;
const _anexar = Element.prototype.append;
Element.prototype.replaceChildren = function (...nos) { return _substituir.call(this, ..._sem_vazios(nos)); };
Element.prototype.append = function (...nos) { return _anexar.call(this, ..._sem_vazios(nos)); };

// Utilidades de interface compartilhadas: elementos, ícones, avisos, campos e tabelas.

export function el(tag, attrs = {}, ...filhos) {
  const node = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (k === 'class') node.className = v;
    else if (k === 'html') node.innerHTML = v;
    else if (k.startsWith('on')) node.addEventListener(k.slice(2), v);
    else if (v !== undefined && v !== null && v !== false) node.setAttribute(k, v === true ? '' : v);
  }
  for (const f of filhos.flat()) {
    if (f === null || f === undefined || f === false) continue;
    node.append(f instanceof Node ? f : document.createTextNode(String(f)));
  }
  return node;
}

// Ícones simples (traço, 24×24) para acompanhar os textos. Sempre usados junto com o nome da ação.
const CAMINHOS = {
  inicio: '<path d="M3 11l9-7 9 7v9a1 1 0 0 1-1 1h-5v-6H9v6H4a1 1 0 0 1-1-1z"/>',
  ordens: '<rect x="5" y="4" width="14" height="17" rx="2"/><path d="M9 2h6v4H9z"/><path d="M9 11h6M9 15h6M9 19h3"/>',
  clientes: '<circle cx="12" cy="8" r="4"/><path d="M4 21c0-4 3.6-6 8-6s8 2 8 6"/>',
  veiculos: '<path d="M5 16l1.6-5a2 2 0 0 1 1.9-1.4h8a2 2 0 0 1 1.9 1.4L19 16"/><rect x="3" y="16" width="18" height="3" rx="1"/><circle cx="7.5" cy="17.5" r="1"/><circle cx="16.5" cy="17.5" r="1"/>',
  equipe: '<circle cx="9" cy="8" r="3.2"/><path d="M3 20c0-3.2 2.7-5 6-5s6 1.8 6 5"/><path d="M16 5a3 3 0 0 1 0 6"/><path d="M18 15c2 .6 3 2.2 3 4.5"/>',
  pecas: '<path d="M4 7l8-4 8 4v10l-8 4-8-4z"/><path d="M4 7l8 4 8-4M12 11v10"/>',
  config: '<circle cx="12" cy="12" r="3"/><circle cx="12" cy="12" r="8"/><path d="M12 2v2M12 20v2M2 12h2M20 12h2"/>',
  camera: '<path d="M4 7h3l2-3h6l2 3h3v12H4z"/><circle cx="12" cy="13" r="4"/>',
  lixeira: '<path d="M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13"/>',
  lapis: '<path d="M4 20h4L19 9l-4-4L4 16z"/>',
  mais: '<path d="M12 5v14M5 12h14"/>',
  ok: '<path d="M5 12l4 4L19 7"/>',
  alerta: '<path d="M12 3l10 18H2z"/><path d="M12 10v5M12 18v.5"/>',
  nuvem: '<path d="M7 18a5 5 0 0 1-.9-9.9A6 6 0 0 1 18 9a4 4 0 0 1 .5 8H7z"/>',
  sair: '<path d="M14 4h4a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2h-4"/><path d="M10 8l-4 4 4 4M6 12h10"/>',
  cadeado: '<rect x="5" y="11" width="14" height="10" rx="2"/><path d="M8 11V7a4 4 0 0 1 8 0v4"/>',
  disco: '<path d="M12 4v11M7 10l5 5 5-5M5 20h14"/>',
  lupa: '<circle cx="11" cy="11" r="7"/><path d="M20 20l-4-4"/>',
  setas: '<path d="M4 12a8 8 0 0 1 14-5.3M20 12a8 8 0 0 1-14 5.3"/><path d="M18 3v4h-4M6 21v-4h4"/>',
  seta: '<path d="M5 12h14M13 6l6 6-6 6"/>',
  pessoa: '<circle cx="12" cy="8" r="4"/><path d="M4 21c0-4 3.6-6 8-6s8 2 8 6"/>',
  chave: '<circle cx="8" cy="15" r="4"/><path d="M11 12l9-9M17 6l3 3"/>',
  x: '<path d="M6 6l12 12M18 6L6 18"/>',
  caixa: '<rect x="3" y="6" width="18" height="13" rx="2"/><path d="M3 10h18M7 15h3"/><path d="M7 6V4h10v2"/>',
  registro: '<path d="M6 3h9l4 4v14H6z"/><path d="M14 3v5h5M9 13l2 2 4-4M9 18h6"/>',
  ajuda: '<circle cx="12" cy="12" r="9"/><path d="M9.5 9.5a2.5 2.5 0 1 1 3.5 2.3c-.7.3-1 .9-1 1.7M12 17v.5"/>',
  carro: '<path d="M4 15l2-6h12l2 6"/><rect x="3" y="15" width="18" height="4" rx="1"/><circle cx="7" cy="17" r="1"/><circle cx="17" cy="17" r="1"/>',
  mais_pessoa: '<circle cx="10" cy="8" r="4"/><path d="M2 21c0-4 3.6-6 8-6s8 2 8 6M19 8v6M16 11h6"/>',
};

export function icone(nome, tamanho = 22) {
  const caminho = CAMINHOS[nome] ?? '';
  return el('span', {
    class: 'icone', 'aria-hidden': 'true',
    html: `<svg width="${tamanho}" height="${tamanho}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round">${caminho}</svg>`,
  });
}

// Botão grande com ícone e texto. `tipo`: primario | secundario | perigo | link
export function botao({ texto, icone: ic, tipo = 'secundario', onclick, type = 'button', title } = {}) {
  return el('button', { type, class: `btn ${tipo}`, onclick, title: title ?? texto },
    ic ? icone(ic, 20) : null, el('span', {}, texto));
}

export function aviso(texto, tipo = 'ok') {
  const area = document.getElementById('avisos');
  const item = el('div', { class: `aviso ${tipo === 'erro' ? 'erro' : ''}`, role: tipo === 'erro' ? 'alert' : 'status' },
    icone(tipo === 'erro' ? 'alerta' : 'ok', 20), el('span', {}, texto));
  area.append(item);
  setTimeout(() => item.remove(), tipo === 'erro' ? 7000 : 4000);
}

export function moeda(valor) {
  return Number(valor ?? 0).toLocaleString('pt-BR', { style: 'currency', currency: 'BRL' });
}

export function confirmar(texto) {
  return window.confirm(texto);
}

// Campo de formulário com rótulo grande. `campo` = { name, label, type, options?, required?, step?, min?, placeholder? }.
export function campo(c, valor = '', erro = '') {
  let input;
  if (c.type === 'select') {
    input = el('select', { name: c.name, required: c.required ? '' : null },
      el('option', { value: '' }, '— escolha —'),
      (c.options ?? []).map(([v, t]) => el('option', { value: v, selected: String(v) === String(valor ?? '') ? '' : null }, t)),
    );
  } else if (c.type === 'textarea') {
    input = el('textarea', { name: c.name, rows: 3, placeholder: c.placeholder }, valor ?? '');
  } else {
    input = el('input', {
      name: c.name,
      type: c.type ?? 'text',
      value: valor ?? '',
      step: c.step,
      min: c.min,
      placeholder: c.placeholder,
      autocomplete: c.autocomplete ?? 'off',
      required: c.required ? '' : null,
    });
  }
  // Sugestões (ex.: cores comuns): lista de escolha que ainda permite digitar outro valor.
  const sugestoes = c.sugestoes ? el('datalist', { id: c.list }, c.sugestoes.map((s) => el('option', { value: s }))) : null;
  if (c.list) input.setAttribute('list', c.list);
  return el('label', { class: 'campo' },
    el('span', { class: 'rotulo' }, c.label),
    input,
    sugestoes,
    c.ajuda ? el('small', { class: 'ajuda' }, c.ajuda) : null,
    erro ? el('span', { class: 'erro-campo' }, erro) : null);
}

export function tabela(colunas, linhas, vazio = 'Nenhum registro cadastrado.') {
  if (!linhas.length) return el('p', { class: 'vazio' }, vazio);
  return el('div', { class: 'tabela-wrap' },
    el('table', {},
      el('thead', {}, el('tr', {}, colunas.map((c) => el('th', {}, c.label)), el('th', {}, 'Ações'))),
      el('tbody', {}, linhas.map((linha) => el('tr', {},
        colunas.map((c) => el('td', { 'data-label': c.label }, c.render ? c.render(linha) : linha[c.key])),
        el('td', { 'data-label': 'Ações', class: 'acoes-linha' }, linha.__acoes ?? ''),
      ))),
    ),
  );
}

export function cabecalhoPagina(titulo, ic, descricao) {
  return el('header', { class: 'cabecalho-pagina' },
    el('h2', {}, icone(ic, 30), el('span', {}, titulo)),
    descricao ? el('p', { class: 'descricao' }, descricao) : null);
}
