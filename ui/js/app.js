// Ponto de entrada: decide a tela certa (ativação → modo → administrador → login → sistema) e monta a casca do sistema.
// Casca no padrão de atendimento simples: cabeçalho escuro, botões grandes, atalhos de teclado (F1 a F9, Esc).
import { invocar } from './api.js';
import { el, aviso, botao, icone } from './components.js';
import { telaAtivacao, telaModo, telaAdministrador, telaLogin } from './telas-entrada.js';
import { render as inicio } from './pages/dashboard.js';
import { render as ordens } from './pages/ordens.js';
import { render as clientesVeiculos } from './pages/clientes_veiculos.js';
import { render as itens } from './pages/itens.js';
import { render as caixa } from './pages/caixa.js';
import { render as funcionarios } from './pages/funcionarios.js';
import { render as configuracoes } from './pages/configuracoes.js';
import { render as registro } from './pages/registro.js';
import { render as venda } from './pages/venda.js';
import { render as fidelidade } from './pages/fidelidade.js';
import { render as orcamentos } from './pages/orcamentos.js';
import { render as financeiro } from './pages/financeiro.js';
import { render as compras } from './pages/compras.js';
import { render as relatorios } from './pages/relatorios.js';
import { render as painel } from './pages/painel.js';

const raiz = document.getElementById('app');

// Telas do sistema. `tecla` é o atalho mostrado nos blocos. `tile` = aparece como bloco na tela inicial.
const TELAS = [
  { id: 'nova_os', titulo: 'Nova Ordem de Serviço', descricao: 'Anotar carro, peças e mão de obra', icone: 'ordens', cor: 'azul', tecla: 'F2', tile: true, pagina: ordens, opcoes: { novo: true } },
  { id: 'carros', titulo: 'Carros na Oficina', descricao: 'Ver serviços em andamento e entregar', icone: 'carro', cor: 'ambar', tecla: 'F3', tile: true, pagina: ordens },
  { id: 'pecas', titulo: 'Peças e Serviços', descricao: 'Preços e estoque das peças', icone: 'pecas', cor: 'verde', tecla: 'F4', tile: true, pagina: itens },
  { id: 'clientes', titulo: 'Clientes e Veículos', descricao: 'Telefones, placas e histórico', icone: 'clientes', cor: 'indigo', tecla: 'F5', tile: true, pagina: clientesVeiculos },
  { id: 'caixa', titulo: 'Caixa', descricao: 'Abrir, receber e fechar o caixa do dia', icone: 'caixa', cor: 'cinza', tecla: 'F6', tile: true, pagina: caixa },
  { id: 'funcionarios', titulo: 'Funcionários', descricao: 'Equipe e acessos ao sistema', icone: 'equipe', cor: 'azul', tecla: 'F7', tile: true, admin: true, pagina: funcionarios },
  { id: 'configuracoes', titulo: 'Configurações', descricao: 'Logo, impressora, backup e rede', icone: 'config', cor: 'cinza', tecla: 'F8', tile: true, admin: true, pagina: configuracoes },
  { id: 'registro', titulo: 'Registro de atividades', descricao: 'Quem mexeu em cada coisa e quando', icone: 'registro', cor: 'cinza', tecla: 'F9', tile: true, admin: true, pagina: registro },
  { id: 'venda', titulo: 'Venda Rápida', descricao: 'Peças e serviços vendidos sem carro', icone: 'caixa', cor: 'verde', tecla: 'F10', tile: true, pagina: venda },
  { id: 'orcamentos', titulo: 'Orçamentos', descricao: 'Propostas e aprovação do cliente', icone: 'ordens', cor: 'verde', tecla: 'F11', tile: true, pagina: orcamentos },
  { id: 'financeiro', titulo: 'Financeiro', descricao: 'Recebimentos, contas a receber e a pagar', icone: 'caixa', cor: 'cinza', tecla: null, tile: true, admin: true, pagina: financeiro },
  { id: 'compras', titulo: 'Compras e Fornecedores', descricao: 'Entrada de peças e fornecedores', icone: 'pecas', cor: 'verde', tecla: null, tile: true, pagina: compras },
  { id: 'painel', titulo: 'Painel da administração', descricao: 'Gráficos de faturamento, recebimentos, contas e estoque', icone: 'registro', cor: 'cinza', tecla: null, tile: true, admin: true, pagina: painel },
  { id: 'relatorios', titulo: 'Relatórios', descricao: 'Faturamento, lucro, serviços e estoque', icone: 'registro', cor: 'cinza', tecla: null, tile: true, admin: true, pagina: relatorios },
  { id: 'fidelidade', titulo: 'Fidelidade', descricao: 'Pontos e extrato por cliente', icone: 'clientes', cor: 'indigo', tecla: null, tile: false, pagina: fidelidade },
];
const INICIO = { id: 'inicio', titulo: 'Início', icone: 'inicio', pagina: inicio };

let estado = null;
let telaAtualId = 'inicio';
let relogio = null;

async function atualizarEstado() {
  estado = await invocar('estado_sistema');
  return estado;
}

// Fluxo de entrada: cada passo só aparece quando o anterior está concluído.
async function telaCerta() {
  await atualizarEstado();
  if (!estado.ativado) return raiz.replaceChildren(telaAtivacao(telaCerta));
  if (!estado.modo) return raiz.replaceChildren(telaModo(estado, telaCerta));
  if (estado.modo !== 'cliente' && !estado.tem_admin) return raiz.replaceChildren(telaAdministrador(telaCerta));
  if (!estado.logado) return raiz.replaceChildren(telaLogin(estado, telaCerta));
  return montarSistema();
}

const ehAdmin = () => estado?.logado?.perfil === 'administrador';

// ---------- Cabeçalho ----------

// Logo da empresa (se cadastrada em Configurações) ou o ícone padrão.
async function marcaDaEmpresa() {
  try {
    const logo = await invocar('logo_obter');
    if (logo) return el('img', { class: 'logo-empresa', src: logo, alt: 'Logo da empresa' });
  } catch (_) { /* sem logo: usa o ícone padrão */ }
  return el('div', { class: 'marca-padrao' }, icone('carro', 36));
}

function atualizarRelogio() {
  const agora = new Date();
  const data = agora.toLocaleDateString('pt-BR', { weekday: 'long', day: '2-digit', month: 'long' });
  const hora = agora.toLocaleTimeString('pt-BR', { hour: '2-digit', minute: '2-digit' });
  const alvo = document.getElementById('relogio');
  if (alvo) alvo.textContent = `${data.charAt(0).toUpperCase() + data.slice(1)} • ${hora}`;
}

function alternarLetra() {
  const grande = document.documentElement.classList.toggle('letra-grande');
  try { localStorage.setItem('letra-grande', grande ? '1' : '0'); } catch (_) { /* sem armazenamento: vale só nesta sessão */ }
  const texto = document.getElementById('texto-letra');
  if (texto) texto.textContent = grande ? 'Letra normal (A-)' : 'Letra maior (A+)';
  aviso(grande ? 'Letras aumentadas.' : 'Tamanho de letra normal.');
}

async function montarSistema() {
  const marca = await marcaDaEmpresa();
  const topo = el('header', { class: 'cabecalho-app' },
    el('div', { class: 'marca-app' },
      marca,
      el('div', {},
        el('h1', {}, 'OFICINA FÁCIL'),
        el('p', { id: 'relogio', class: 'relogio' }, ''))),
    el('div', { class: 'acoes-app' },
      el('button', { type: 'button', id: 'btn-inicio', class: 'botao-topo escondido', onclick: () => irPara('inicio'), 'aria-label': 'Voltar para a tela inicial (Esc)' },
        icone('inicio', 26), el('span', {}, 'Voltar ao início')),
      el('button', { type: 'button', class: 'botao-topo', onclick: alternarLetra, 'aria-label': 'Aumentar ou diminuir o tamanho das letras' },
        icone('ordens', 24), el('span', { id: 'texto-letra' }, document.documentElement.classList.contains('letra-grande') ? 'Letra normal (A-)' : 'Letra maior (A+)')),
      el('button', { type: 'button', class: 'botao-topo', onclick: () => abrirAjuda(), 'aria-label': 'Abrir a ajuda com os atalhos (F1)' },
        icone('ajuda', 26), el('span', {}, 'Ajuda (F1)'))));

  const barraUsuario = el('div', { class: 'barra-usuario' },
    el('span', { class: 'quem' }, icone('pessoa', 22), `${estado.logado.nome ?? estado.logado.usuario} · ${ehAdmin() ? 'Administrador' : 'Funcionário'}`),
    el('span', { class: 'quem' }, `Computador: ${estado.computador}`),
    botao({ texto: 'Sair', icone: 'sair', onclick: async () => { await invocar('logout'); telaCerta(); } }));

  const conteudo = el('main', { id: 'conteudo', class: 'conteudo' });
  raiz.replaceChildren(el('div', { class: 'casca' }, topo, barraUsuario, el('div', { id: 'alerta' }), conteudo));
  atualizarRelogio();
  // Trocar a logo em Configurações atualiza o topo na hora.
  window.addEventListener('logo-mudou', async () => {
    const nova = await marcaDaEmpresa();
    document.querySelector('.marca-app')?.replaceChildren(nova, document.querySelector('.marca-app > div'));
  });
  clearInterval(relogio);
  relogio = setInterval(atualizarRelogio, 15000);
  desenharAlerta();
  setInterval(() => { atualizarEstado().then(desenharAlerta).catch(() => {}); }, 5000);
  irPara('inicio');
}

// ---------- Faixas de situação (rede, offline, pendências) ----------

function desenharAlerta() {
  const area = document.getElementById('alerta');
  if (!area || !estado) return;
  const barras = [];
  const a = estado.alerta;
  if (a) {
    barras.push(el('div', { class: `faixa ${a.tipo === 'offline' || a.tipo === 'isolado' ? 'info' : 'alerta'}`, role: 'alert' },
      icone('alerta', 26), el('p', {}, a.texto),
      a.tipo === 'sem_servidor' || a.tipo === 'sem_clientes'
        ? botao({ texto: 'Trabalhar em modo offline', icone: 'nuvem', tipo: 'primario', onclick: async () => {
          await invocar('trabalhar_offline', { ligar: true }); aviso('Modo offline ligado. As alterações serão enviadas depois.'); await atualizarEstado(); desenharAlerta();
        } })
        : null));
  }
  if (estado.pendentes > 0 && estado.modo === 'cliente') {
    barras.push(el('div', { class: 'faixa info' }, icone('nuvem', 26),
      el('p', {}, `${estado.pendentes} alteração(ões) aguardando envio ou conflito. Veja em Configurações.`)));
  }
  area.replaceChildren(...barras);
}

// ---------- Navegação ----------

function telaPorId(id) {
  if (id === 'inicio') return INICIO;
  return TELAS.find((t) => t.id === id);
}

// Telas que o usuário pode abrir (administrador vê tudo; funcionário, só as comuns).
const telasDoUsuario = () => TELAS.filter((t) => !t.admin || ehAdmin());

async function irPara(id, opcoes = {}) {
  const tela = telaPorId(id);
  if (!tela) return;
  if (tela.admin && !ehAdmin()) return aviso('Só o administrador pode abrir esta tela.', 'erro');
  telaAtualId = id;
  document.getElementById('btn-inicio')?.classList.toggle('escondido', id === 'inicio');
  const conteudo = document.getElementById('conteudo');
  if (!conteudo) return;
  conteudo.scrollTop = 0;
  window.scrollTo({ top: 0 });
  try {
    await tela.pagina(conteudo, { irPara, atualizarEstado, estado: () => estado, admin: ehAdmin(), telas: telasDoUsuario(), opcoes });
  } catch (e) {
    aviso(e.texto ?? String(e), 'erro');
  }
}

// ---------- Atalhos de teclado e ajuda ----------

// F1 Ajuda · F2–F9 telas (ver TELAS) · Esc volta ao início (ou fecha a ajuda).
document.addEventListener('keydown', (e) => {
  if (!estado?.logado) return;
  if (e.key === 'Escape') {
    if (fecharAjuda()) return e.preventDefault();
    if (telaAtualId !== 'inicio') { e.preventDefault(); irPara('inicio'); }
    return;
  }
  if (e.key === 'F1') { e.preventDefault(); return abrirAjuda(); }
  // Atalho rápido: Ctrl+F4 abre direto o cadastro de peça no estoque.
  if (e.ctrlKey && e.key === 'F4') {
    e.preventDefault();
    return irPara('pecas', { novo: true });
  }
  const tela = TELAS.find((t) => t.tecla === e.key);
  if (tela) {
    e.preventDefault();
    if (tela.admin && !ehAdmin()) return aviso('Só o administrador pode abrir esta tela.', 'erro');
    irPara(tela.id, {});
  }
});

function abrirAjuda() {
  if (document.getElementById('ajuda')) return;
  const linhas = [
    ['F1', 'Abre esta ajuda'],
    ...TELAS.filter((t) => t.tile && (!t.admin || ehAdmin())).map((t) => [t.tecla, t.titulo]),
    ['Esc', 'Volta para a tela inicial'],
  ];
  const modal = el('div', { id: 'ajuda', class: 'sobreposicao', role: 'dialog', 'aria-modal': 'true', 'aria-labelledby': 'ajuda-titulo' },
    el('div', { class: 'janela' },
      el('h2', { id: 'ajuda-titulo' }, icone('ajuda', 32), 'Como usar o sistema'),
      el('p', { class: 'texto-grande' }, 'Toque nos blocos da tela inicial ou use as teclas abaixo.'),
      el('table', { class: 'tabela-atalhos' }, el('tbody', {},
        linhas.map(([tecla, texto]) => el('tr', {}, el('th', {}, el('kbd', {}, tecla)), el('td', {}, texto))))),
      el('div', { class: 'acoes' }, botao({ texto: 'Entendi, fechar', icone: 'ok', tipo: 'primario', onclick: fecharAjuda }))));
  document.body.append(modal);
}

function fecharAjuda() {
  const modal = document.getElementById('ajuda');
  if (!modal) return false;
  modal.remove();
  return true;
}

// Preferência de letra grande, guardada neste computador.
try {
  if (localStorage.getItem('letra-grande') === '1') document.documentElement.classList.add('letra-grande');
} catch (_) { /* sem armazenamento: começa na letra normal */ }

telaCerta();
