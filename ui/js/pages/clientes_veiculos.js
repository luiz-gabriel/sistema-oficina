// Clientes e Veículos numa só tela: duas abas grandes, cada uma com a tela já existente.
import { el, botao } from '../components.js';
import { render as clientes } from './clientes.js';
import { render as veiculos } from './veiculos.js';

const ABAS = [
  { id: 'clientes', titulo: 'Clientes', icone: 'clientes', pagina: clientes },
  { id: 'veiculos', titulo: 'Veículos', icone: 'veiculos', pagina: veiculos },
];

export async function render(container, ctx) {
  const area = el('div', { class: 'area-aba' });
  const barra = el('div', { class: 'abas', role: 'tablist' });
  const botoes = ABAS.map((a) => {
    const b = botao({ texto: a.titulo, icone: a.icone, onclick: () => abrir(a.id) });
    b.dataset.id = a.id;
    b.setAttribute('role', 'tab');
    return b;
  });
  barra.append(...botoes);

  async function abrir(id) {
    botoes.forEach((b) => {
      const ativa = b.dataset.id === id;
      b.classList.toggle('ativa', ativa);
      b.setAttribute('aria-selected', String(ativa));
    });
    const aba = ABAS.find((a) => a.id === id);
    try {
      await aba.pagina(area, ctx);
    } catch (e) {
      area.replaceChildren(el('p', { class: 'faixa alerta' }, e.texto ?? String(e)));
    }
  }

  container.replaceChildren(barra, area);
  await abrir(ctx.opcoes?.aba ?? 'clientes');
}
