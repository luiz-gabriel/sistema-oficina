// Dispositivos: aparelhos autorizados a entrar no sistema, pedidos de pareamento e revogação. Tela só do administrador.
// O pareamento por QR Code ainda não existe: o celular/tablet aparece como marcador. O PC pareia pelo código de 6 dígitos.
import { chamar, invocar } from '../api.js';
import { el, aviso, tabela, botao, icone, cabecalhoPagina, confirmar } from '../components.js';

const INTERVALO_PEDIDOS_MS = 4000;

function quando(iso) {
  if (!iso) return '—';
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? '—' : d.toLocaleString('pt-BR', { dateStyle: 'short', timeStyle: 'short' });
}

// Endereço (IP:porta) do servidor web deste computador. No navegador comum, usa o endereço da própria página.
async function enderecoDoServidor() {
  try {
    const situacao = await invocar('web_status');
    const e = situacao?.endereco;
    if (e?.ip) return `${e.ip}:${e.porta}`;
  } catch {
    // Navegador comum: sem acesso à ponte do aplicativo.
  }
  return window.location.host || 'endereço indisponível';
}

export async function render(container, { estado }) {
  if (estado().logado?.perfil !== 'administrador') {
    container.replaceChildren(cabecalhoPagina('Dispositivos', 'equipe', 'Somente o administrador gerencia os aparelhos.'));
    return;
  }

  const escolhaArea = el('section', { class: 'card' });
  const pedidosArea = el('section', { class: 'card' });
  const listaArea = el('section', { class: 'card' });
  container.replaceChildren(
    cabecalhoPagina('Dispositivos', 'equipe',
      'Aparelhos que podem entrar no sistema. O limite é de 10, contando este computador principal.'),
    escolhaArea, pedidosArea, listaArea,
  );

  // Atualiza os pedidos enquanto a tela estiver aberta. Para quando a tela sai do documento.
  let temporizador = null;
  const parar = () => { if (temporizador) { clearInterval(temporizador); temporizador = null; } };

  function mostrarEscolha() {
    escolhaArea.replaceChildren(
      el('h3', {}, icone('mais', 22), 'Adicionar dispositivo'),
      el('p', {}, 'Qual aparelho você quer configurar?'),
      el('div', { class: 'acoes' },
        botao({ texto: 'Computador (PC)', icone: 'equipe', tipo: 'primario', onclick: () => configurarPc() }),
        botao({ texto: 'Celular ou tablet', icone: 'equipe', onclick: () => configurarCelular() }),
      ),
    );
  }

  async function configurarPc() {
    try {
      const pareamento = await chamar('dispositivos_iniciar_pareamento');
      const endereco = await enderecoDoServidor();
      const expira = new Date(pareamento.expira_em * 1000).toLocaleTimeString('pt-BR', { timeStyle: 'short' });
      escolhaArea.replaceChildren(
        el('h3', {}, icone('chave', 22), 'Pareamento de computador'),
        el('p', {}, 'No outro computador, abra o sistema, informe o endereço e o código abaixo.'),
        el('dl', { class: 'pareamento' },
          el('dt', {}, 'Endereço do servidor'), el('dd', { class: 'destaque' }, endereco),
          el('dt', {}, 'Código de 6 dígitos'), el('dd', { class: 'destaque codigo' }, pareamento.codigo),
        ),
        el('p', { class: 'ajuda' }, `O código vale até ${expira} e serve uma vez. Depois de digitado, o pedido aparece abaixo para você aprovar.`),
        el('div', { class: 'acoes' }, botao({ texto: 'Fechar', icone: 'x', onclick: () => mostrarEscolha() })),
      );
    } catch (e) { aviso(e.texto, 'erro'); }
  }

  function configurarCelular() {
    escolhaArea.replaceChildren(
      el('h3', {}, icone('mais', 22), 'Pareamento de celular ou tablet'),
      el('p', { class: 'marcador' }, 'QR Code: em seguida'),
      el('div', { class: 'acoes' }, botao({ texto: 'Voltar', icone: 'x', onclick: () => mostrarEscolha() })),
    );
  }

  async function carregarPedidos() {
    if (!container.isConnected) return parar();
    try {
      const pedidos = await chamar('pareamento_listar_pedidos');
      pedidosArea.replaceChildren(
        el('h3', {}, icone('chave', 22), pedidos.length ? `Pedidos aguardando aprovação (${pedidos.length})` : 'Pedidos aguardando aprovação'),
        pedidos.length
          ? tabela([
            { key: 'nome_aparelho', label: 'Aparelho' },
            { key: 'criado_em', label: 'Pedido em', render: (p) => quando(p.criado_em) },
          ], pedidos.map((p) => ({
            ...p,
            __acoes: el('span', { class: 'botoes-linha' },
              botao({ texto: 'Aprovar', icone: 'ok', tipo: 'link', onclick: async () => {
                try { await chamar('pareamento_aprovar', { id: p.id }); aviso('Aparelho autorizado.'); await carregar(); }
                catch (e) { aviso(e.texto, 'erro'); }
              } }),
              botao({ texto: 'Recusar', icone: 'x', tipo: 'link perigo', onclick: async () => {
                if (!confirmar(`Recusar o pedido de ${p.nome_aparelho}?`)) return;
                try { await chamar('pareamento_recusar', { id: p.id }); await carregar(); }
                catch (e) { aviso(e.texto, 'erro'); }
              } }),
            ),
          })), '')
          : el('p', { class: 'vazio' }, 'Nenhum pedido no momento.'),
      );
    } catch (e) { aviso(e.texto, 'erro'); }
  }

  async function carregarLista() {
    try {
      const dispositivos = await chamar('dispositivos_listar');
      const ativos = dispositivos.filter((d) => d.situacao === 'ATIVO').length;
      const linhas = dispositivos.map((d) => ({
        ...d,
        nome_exibicao: d.principal ? `${d.nome} (este computador)` : d.nome,
        situacao_texto: d.situacao === 'ATIVO' ? 'Ativo' : 'Revogado',
        __acoes: d.situacao === 'ATIVO'
          ? botao({ texto: 'Revogar', icone: 'x', tipo: 'link perigo', onclick: async () => {
            const texto = d.principal
              ? `Revogar ${d.nome}? Este computador deixa de aceitar login. Só é permitido se outro aparelho autorizado e um administrador continuarem com acesso.`
              : `Revogar ${d.nome}? O aparelho perde o acesso e as sessões dele são encerradas.`;
            if (!confirmar(texto)) return;
            try { await chamar('dispositivos_revogar', { id: d.id }); aviso('Dispositivo revogado.'); await carregarLista(); }
            catch (e) { aviso(e.texto, 'erro'); }
          } })
          : null,
      }));
      listaArea.replaceChildren(
        el('h3', {}, icone('equipe', 22), `Autorizados (${ativos} de 10 vagas usadas)`),
        tabela([
          { key: 'nome_exibicao', label: 'Nome' },
          { key: 'autorizado_em', label: 'Autorizado em', render: (d) => quando(d.autorizado_em) },
          { key: 'ultimo_acesso', label: 'Último acesso', render: (d) => quando(d.ultimo_acesso) },
          { key: 'situacao_texto', label: 'Situação' },
        ], linhas, 'Nenhum dispositivo autorizado ainda.'),
      );
    } catch (e) { aviso(e.texto, 'erro'); }
  }

  async function carregar() {
    await Promise.all([carregarPedidos(), carregarLista()]);
  }

  mostrarEscolha();
  await carregar();
  temporizador = setInterval(carregarPedidos, INTERVALO_PEDIDOS_MS);
}
