// Registro de atividades: quem mexeu em cada coisa, em que computador e quando. Só o administrador vê.
import { invocar } from '../api.js';
import { el, icone, cabecalhoPagina, tabela } from '../components.js';

const ROTULOS = {
  'entrou no sistema': 'Entrou no sistema',
  'saiu do sistema': 'Saiu do sistema',
  'configurou o modo de funcionamento': 'Mudou o modo de funcionamento',
  'configurou a impressora térmica': 'Mudou a impressora',
  'trocou a logo da empresa': 'Trocou a logo',
  'removeu a logo da empresa': 'Removeu a logo',
  'resolveu conflito de sincronização': 'Resolveu conflito de sincronização',
  'ligou o modo offline': 'Ligou o modo offline',
  'voltou a sincronizar': 'Voltou a sincronizar',
};

const NOMES = {
  clientes: 'cliente', veiculos: 'veículo', funcionarios: 'funcionário', itens: 'peça/serviço',
  ordens: 'ordem de serviço', caixa: 'caixa', fotos: 'foto',
};

// Traduz o comando técnico (ex.: "clientes_criar") para uma frase simples.
function descrever(acao) {
  if (ROTULOS[acao]) return ROTULOS[acao];
  const m = acao.match(/^(clientes|veiculos|funcionarios|itens|ordens|caixa|veiculos_foto)_(\w+)$/);
  if (!m) return acao.replace(/_/g, ' ');
  const verbos = {
    criar: 'Cadastrou', atualizar: 'Alterou', remover: 'Removeu', excluir: 'Excluiu', dar_acesso: 'Deu acesso a',
    revogar_acesso: 'Retirou o acesso de', alterar_status: 'Mudou a situação de', substituir_itens: 'Alterou os itens de',
    abrir: 'Abriu', fechar: 'Fechou', adicionar: 'Adicionou', foto_adicionar: 'Adicionou foto em',
  };
  const verbo = verbos[m[2]] ?? 'Alterou';
  const alvo = m[1] === 'veiculos_foto' ? 'foto de veículo' : NOMES[m[1]] ?? m[1];
  return `${verbo} ${alvo}`;
}

export async function render(container) {
  const lista = await invocar('registro_listar', { limite: 300 });
  const busca = el('input', { type: 'search', placeholder: 'Buscar por usuário, computador ou ação…', 'aria-label': 'Buscar no registro' });
  const area = el('div', {});

  const desenhar = () => {
    const termo = busca.value.trim().toLowerCase();
    const filtradas = lista
      .filter((e) => !termo || `${e.usuario} ${e.computador} ${descrever(e.acao)} ${e.detalhe}`.toLowerCase().includes(termo))
      .map((e) => ({
        ...e,
        quando: new Date(e.criado_em).toLocaleString('pt-BR'),
        o_que: descrever(e.acao),
        detalhe_texto: e.detalhe && e.detalhe !== '{}' ? e.detalhe : '—',
      }));
    area.replaceChildren(tabela([
      { key: 'quando', label: 'Quando' },
      { key: 'usuario', label: 'Quem' },
      { key: 'computador', label: 'Computador' },
      { key: 'o_que', label: 'O que foi feito' },
      { key: 'detalhe_texto', label: 'Detalhes' },
    ], filtradas, termo ? 'Nada encontrado com esse termo.' : 'Nenhuma atividade registrada ainda.'));
  };
  busca.addEventListener('input', desenhar);

  container.replaceChildren(
    cabecalhoPagina('Registro de atividades', 'registro', 'Cada cadastro, alteração, pagamento e configuração fica anotado aqui. Só o administrador vê esta tela.'),
    el('section', { class: 'card' },
      el('label', { class: 'campo' }, el('span', { class: 'rotulo' }, icone('lupa', 22), 'Procurar'), busca),
      el('p', { class: 'ajuda' }, `Mostrando as últimas ${lista.length} atividades.`),
      area),
  );
  desenhar();
}
