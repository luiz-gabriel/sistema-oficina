// Atualiza a tela atual sem recarregar o aplicativo (assim a mensagem de confirmação continua visível).
const reabrirPagina = () => document.querySelector('.item-menu.ativo')?.click();

// Configurações (administrador): modo de funcionamento em rede, modo offline, pendências, backup e histórico.
import { invocar } from '../api.js';
import { el, aviso, botao, icone, cabecalhoPagina, tabela, confirmar } from '../components.js';

const MODOS = [
  { valor: 'local', titulo: 'Só este computador', icone: 'inicio', texto: 'Sem rede. Os dados ficam somente neste computador.' },
  { valor: 'servidor', titulo: 'Computador principal da oficina', icone: 'config', texto: 'Guarda os dados. Os outros computadores da oficina se conectam a ele pela rede.' },
  { valor: 'cliente', titulo: 'Usar o computador principal', icone: 'setas', texto: 'Usa os dados do computador principal. Funciona também offline, enviando depois.' },
];

const STATUS_TEXTO = {
  local: 'Local', pendente: 'Aguardando envio', sincronizado: 'Enviado', conflito: 'Conflito', erro: 'Erro', descartado: 'Descartado',
};

export async function render(container) {
  const c = await invocar('config_obter');
  const area = el('div', { class: 'config' });
  container.replaceChildren(cabecalhoPagina('Configurações', 'config',
    `Este computador: ${c.computador}${c.ip_local ? ` · endereço na rede: ${c.ip_local}:${c.porta}` : ''}`), area);

  // Hora do computador: mostra se a ativação usou hora atômica e o desvio do relógio.
  const horaInfo = c.fonte_hora === 'ntp'
    ? `Data da ativação conferida pela hora atômica (relógio do computador ${Number(c.desvio_hora_segundos ?? 0) >= 0 ? 'adiantado' : 'atrasado'} em ${Math.abs(Number(c.desvio_hora_segundos ?? 0)).toFixed(1)} s).`
    : c.fonte_hora === 'relogio_local'
      ? 'A data da ativação veio do relógio deste computador (sem internet). Confira a data e a hora do sistema.'
      : null;
  area.append(
    horaInfo ? el('section', { class: 'card' }, el('p', { class: 'ajuda' }, horaInfo)) : null,
    secaoModo(c),
    await secaoLogo(),
    await secaoOffline(c),
    await secaoPendencias(),
    await secaoCopiasLocais(),
    await secaoImpressora(),
    await secaoNuvem(),
    await secaoHistorico(),
  );
}

// Logo da empresa: aparece no topo do sistema. Só o administrador troca.
async function secaoLogo() {
  const atual = await invocar('logo_obter').catch(() => '');
  const previa = atual
    ? el('div', { class: 'logo-previa' }, el('img', { src: atual, alt: 'Logo atual da empresa' }))
    : el('p', { class: 'ajuda' }, 'Nenhuma logo cadastrada. O sistema mostra o ícone padrão.');
  const arquivo = el('input', { type: 'file', accept: 'image/png,image/jpeg,image/webp', 'aria-label': 'Escolher a imagem da logo' });
  return secao('Logo da empresa', 'camera',
    el('p', { class: 'ajuda' }, 'A logo aparece no topo do sistema. Use PNG, JPG ou WEBP de até 300 KB.'),
    previa,
    el('label', { class: 'campo' }, el('span', { class: 'rotulo' }, 'Escolher imagem'), arquivo),
    el('div', { class: 'acoes' },
      botao({ texto: 'Salvar logo', icone: 'ok', tipo: 'primario', onclick: async () => {
        const f = arquivo.files?.[0];
        if (!f) return aviso('Escolha uma imagem primeiro.', 'erro');
        if (f.size > 300 * 1024) return aviso('A imagem é grande demais. Use uma de até 300 KB.', 'erro');
        try {
          const dados = await lerComoDataUrl(f);
          await invocar('logo_salvar', { dados });
          aviso('Logo salva.');
          window.dispatchEvent(new Event('logo-mudou'));
          reabrirPagina();
        } catch (e) { aviso(e.texto ?? String(e), 'erro'); }
      } }),
      atual ? botao({ texto: 'Remover logo', icone: 'lixeira', onclick: async () => {
        if (!confirmar('Remover a logo da empresa?')) return;
        try {
          await invocar('logo_salvar', { dados: '' });
          aviso('Logo removida.');
          window.dispatchEvent(new Event('logo-mudou'));
          reabrirPagina();
        } catch (e) { aviso(e.texto, 'erro'); }
      } }) : null));
}

function lerComoDataUrl(arq) {
  return new Promise((ok, falha) => {
    const leitor = new FileReader();
    leitor.onload = () => ok(leitor.result);
    leitor.onerror = () => falha(leitor.error);
    leitor.readAsDataURL(arq);
  });
}

// Impressora térmica deste computador (cada computador tem a sua).
async function secaoImpressora() {
  const atual = await invocar('impressora_atual').catch(() => '');
  const destino = el('input', { value: atual, placeholder: 'Ex.: 192.168.0.50:9100 ou /dev/usb/lp0', 'aria-label': 'Endereço da impressora' });
  return secao('Impressora térmica', 'config',
    el('p', { class: 'ajuda' }, 'Impressora de cupom de 80 mm. Pela rede, use o endereço e a porta 9100 (ex.: 192.168.0.50:9100). Ligada direto no computador por USB, use o caminho do dispositivo (ex.: /dev/usb/lp0). Deixe vazio para desligar.'),
    el('label', { class: 'campo' }, el('span', { class: 'rotulo' }, 'Endereço ou caminho da impressora'), destino),
    el('div', { class: 'acoes' },
      botao({ texto: 'Salvar', icone: 'ok', tipo: 'primario', onclick: async () => {
        try { await invocar('impressora_salvar', { destino: destino.value }); aviso('Impressora salva.'); }
        catch (e) { aviso(e.texto, 'erro'); }
      } }),
      botao({ texto: 'Imprimir página de teste', icone: 'disco', onclick: async () => {
        try { await invocar('impressora_testar'); aviso('Página de teste enviada.'); }
        catch (e) { aviso(e.texto, 'erro'); }
      } })));
}

function secao(titulo, ic, ...filhos) {
  return el('section', { class: 'card secao' }, el('h3', {}, icone(ic, 24), titulo), ...filhos);
}

function secaoModo(c) {
  let escolhido = c.modo || 'local';
  let endereco = c.servidor_addr ?? '';
  const opcoes = el('div', { class: 'opcoes' });
  const extra = el('div', {});
  const desenhar = () => {
    opcoes.replaceChildren(...MODOS.map((m) => el('button', {
      type: 'button', class: `opcao ${escolhido === m.valor ? 'escolhida' : ''}`, 'aria-pressed': String(escolhido === m.valor),
      onclick: () => { escolhido = m.valor; desenhar(); },
    }, icone(m.icone, 30), el('strong', {}, m.titulo), el('span', {}, m.texto))));
    extra.replaceChildren(escolhido === 'cliente' ? blocoCliente() : escolhido === 'servidor' ? aviso_servidor() : '');
  };
  function aviso_servidor() {
    return el('p', { class: 'faixa info' }, icone('alerta', 24),
      el('span', {}, `Outros computadores devem usar o endereço ${c.ip_local ?? 'deste computador'}:${c.porta}. Este computador precisa ficar ligado para a oficina funcionar em rede.`));
  }
  function blocoCliente() {
    const campoEnd = el('input', { value: endereco, placeholder: 'Ex.: 192.168.0.10:7878', 'aria-label': 'Endereço do computador principal', 'data-ref': 'endereco' });
    const lista = el('div', { class: 'achados' });
    return el('div', { class: 'bloco-cliente' },
      el('label', { class: 'campo' }, el('span', { class: 'rotulo' }, 'Endereço do computador principal'), campoEnd),
      botao({ texto: 'Procurar na rede', icone: 'lupa', onclick: async () => {
        lista.replaceChildren(el('p', {}, 'Procurando...'));
        try {
          const achados = await invocar('rede_buscar');
          lista.replaceChildren(...(achados.length
            ? achados.map((a) => botao({ texto: `${a.computador} — ${a.endereco} (${a.papel === 'servidor' ? 'principal' : 'cliente'})`, onclick: () => { campoEnd.value = a.endereco; } }))
            : [el('p', {}, 'Nenhum computador encontrado. Confira se o principal está ligado e na mesma rede.')]));
        } catch (e) { aviso(e.texto, 'erro'); }
      } }),
      lista);
  }
  desenhar();
  const modoAtual = c.modo ? MODOS.find((m) => m.valor === c.modo)?.titulo : null;
  return secao('Como este computador funciona', 'config',
    el('p', { class: 'texto-grande' }, modoAtual
      ? `Modo atual deste computador: ${modoAtual}.`
      : 'Ainda não foi escolhido um modo para este computador. Escolha uma opção abaixo e salve.'),
    el('p', { class: 'ajuda' }, 'Para trocar, toque na opção desejada e depois em "Salvar modo de funcionamento". A troca só pode ser feita pelo administrador. Ao escolher "Usar o computador principal", este computador passa a usar os dados do principal, que são copiados para cá; por isso informe o endereço dele.'),
    opcoes, extra,
    el('div', { class: 'acoes' }, botao({ texto: 'Salvar modo de funcionamento', icone: 'ok', tipo: 'primario', onclick: async () => {
      const end = area_endereco();
      try {
        await invocar('config_modo', { modo: escolhido, endereco: escolhido === 'cliente' ? end : null });
        aviso('Modo de funcionamento salvo.');
        reabrirPagina();
      } catch (e) { aviso(e.texto, 'erro'); }
    } })));
  function area_endereco() {
    return document.querySelector('[data-ref="endereco"]')?.value?.trim() ?? '';
  }
}

async function secaoOffline(c) {
  const estado = await invocar('estado_sistema');
  const txt = c.offline
    ? (c.modo === 'cliente' ? 'Ligado: as alterações ficam guardadas aqui e serão enviadas quando a rede voltar.' : 'Ligado: este computador não atua como principal até você voltar à rede.')
    : 'Desligado: o sistema está ligado à rede da oficina.';
  return secao('Rede e modo offline', 'nuvem',
    el('p', { class: 'texto-grande' }, txt),
    el('p', {}, `Alterações aguardando envio ou com conflito: ${estado.pendentes}.`),
    el('div', { class: 'acoes' },
      c.offline
        ? botao({ texto: 'Voltar a sincronizar agora', icone: 'setas', tipo: 'primario', onclick: async () => {
          try { await invocar('trabalhar_offline', { ligar: false }); aviso('Sincronizando com a rede...'); reabrirPagina(); } catch (e) { aviso(e.texto, 'erro'); }
        } })
        : botao({ texto: 'Trabalhar em modo offline', icone: 'nuvem', onclick: async () => {
          if (!confirmar('Trabalhar offline? As alterações ficam guardadas aqui até a rede voltar.')) return;
          await invocar('trabalhar_offline', { ligar: true }); aviso('Modo offline ligado.'); reabrirPagina();
        } }),
      botao({ texto: 'Sincronizar agora', icone: 'setas', onclick: async () => {
        try { await invocar('sincronizar_agora'); aviso('Sincronizado.'); reabrirPagina(); } catch (e) { aviso(e.texto, 'erro'); }
      } })));
}

async function secaoPendencias() {
  const lista = await invocar('pendencias');
  const linhas = lista.map((a) => ({
    ...a,
    o_que: `${a.comando.replace(/_/g, ' ')}${a.args?.dados?.nome ? ` — ${a.args.dados.nome}` : a.args?.dados?.placa ? ` — ${a.args.dados.placa}` : ''}`,
    status_texto: el('span', { class: `status-chip ${a.status}` }, STATUS_TEXTO[a.status] ?? a.status),
    __acoes: a.status === 'conflito' ? el('span', { class: 'botoes-linha' },
      botao({ texto: 'Manter a versão do servidor', icone: 'disco', tipo: 'link', onclick: () => resolver(a.uuid, 'servidor') }),
      botao({ texto: 'Usar a minha versão', icone: 'ok', tipo: 'link', onclick: () => resolver(a.uuid, 'local') })) : null,
  }));
  return secao('Alterações pendentes e conflitos', 'alerta',
    el('p', { class: 'ajuda' }, 'Um conflito acontece quando a mesma informação foi mudada aqui e no computador principal. Nada é trocado sem a sua escolha.'),
    tabela([
      { key: 'criado_em', label: 'Quando' },
      { key: 'usuario', label: 'Usuário' },
      { key: 'o_que', label: 'O que foi feito' },
      { key: 'status_texto', label: 'Situação', render: (a) => a.status_texto },
      { key: 'erro', label: 'Detalhe' },
    ], linhas, 'Nenhuma alteração pendente. Tudo está sincronizado.'));
}

async function resolver(uuid, escolha) {
  try {
    await invocar('resolver_conflito', { uuid, escolha });
    aviso('Conflito resolvido.');
    reabrirPagina();
  } catch (e) { aviso(e.texto, 'erro'); }
}

function secaoBackup() {
  const caminho = el('input', { placeholder: 'Ex.: /home/usuario/Google Drive/Oficina', 'aria-label': 'Pasta de destino' });
  return secao('Backup e cópia dos dados', 'disco',
    el('p', { class: 'ajuda' }, 'Uma cópia do banco é feita automaticamente a cada 6 horas, na pasta de dados do sistema.'),
    botao({ texto: 'Fazer cópia agora', icone: 'disco', tipo: 'primario', onclick: async () => {
      try { const f = await invocar('backup_agora'); aviso(`Cópia criada: ${f}`); } catch (e) { aviso(e.texto, 'erro'); }
    } }),
    el('label', { class: 'campo' }, el('span', { class: 'rotulo' }, 'Exportar os dados para uma pasta'), caminho,
      el('small', { class: 'ajuda' }, 'Para enviar ao Google Drive, escolha uma pasta que esteja sincronizada com o Google Drive neste computador.')),
    botao({ texto: 'Exportar agora', icone: 'nuvem', onclick: async () => {
      if (!caminho.value.trim()) { aviso('Informe a pasta de destino.', 'erro'); return; }
      try { const f = await invocar('exportar_dados', { caminho: caminho.value.trim() }); aviso(`Exportado: ${f}`); } catch (e) { aviso(e.texto, 'erro'); }
    } }));
}

async function secaoHistorico() {
  const lista = await invocar('historico', { limite: 30 });
  const linhas = lista.map((a) => ({
    ...a,
    quando: new Date(a.criado_em.replace(' ', 'T') + 'Z').toLocaleString('pt-BR'),
    o_que: a.comando.replace(/_/g, ' '),
    status_texto: el('span', { class: `status-chip ${a.status}` }, STATUS_TEXTO[a.status] ?? a.status),
  }));
  return secao('Histórico de alterações', 'ordens',
    el('p', { class: 'ajuda' }, 'Cada alteração tem registro de computador, usuário, data e situação de envio.'),
    tabela([
      { key: 'quando', label: 'Quando' },
      { key: 'usuario', label: 'Usuário' },
      { key: 'computador', label: 'Computador' },
      { key: 'o_que', label: 'Alteração' },
      { key: 'status_texto', label: 'Situação', render: (a) => a.status_texto },
    ], linhas, 'Nenhuma alteração registrada.'));
}

// ---------------------------------------------------------------------------
// Backup no Google Drive (só administrador). A conta Google é a do próprio usuário.
// ---------------------------------------------------------------------------

const PASSOS_GOOGLE = [
  'Acesse console.cloud.google.com com a conta Google da oficina e crie um projeto (ex.: "Oficina backup").',
  'No menu, vá em "APIs e serviços" > "Biblioteca", procure "Google Drive API" e clique em "Ativar".',
  'Em "APIs e serviços" > "Tela de permissão OAuth", escolha "Externo", preencha o nome do aplicativo e seu e-mail. Clique em "Publicar aplicativo" (sem isso, o Google desativa a conexão a cada 7 dias).',
  'Em "Credenciais" > "Criar credenciais" > "ID do cliente OAuth", escolha o tipo "App para computador". Copie o Client ID e o Client Secret e cole nos campos abaixo.',
];

function textoEstadoNuvem(st) {
  if (st.conexao === 'aguardando') return 'Aguardando a autorização no navegador...';
  if (st.conectado) return st.email ? `Conectado como ${st.email}` : 'Conectado ao Google Drive.';
  if (!st.configurado) return 'Salve o Client ID e o Client Secret para poder conectar.';
  return 'Não conectado.';
}

async function secaoNuvem() {
  let st;
  try {
    st = await invocar('nuvem_status');
  } catch (e) {
    return secao('Backup no Google Drive', 'nuvem', el('p', { class: 'ajuda' }, e.texto));
  }

  const idInput = el('input', { value: st.client_id ?? '', placeholder: 'Ex.: 123456-abc.apps.googleusercontent.com', 'aria-label': 'Client ID' });
  const segredoInput = el('input', {
    type: 'password', autocomplete: 'off', 'aria-label': 'Client Secret',
    placeholder: st.configurado ? '(já salvo; deixe em branco para manter)' : 'Cole o Client Secret',
  });
  const manterInput = el('input', { type: 'number', min: 1, max: 100, value: String(st.manter ?? 10), 'aria-label': 'Manter as últimas cópias' });
  const estado = el('p', { class: 'ajuda', role: 'status' }, textoEstadoNuvem(st));
  const lista = el('div', { class: 'nuvem-lista' }, el('p', { class: 'ajuda' }, st.conectado ? 'Carregando...' : 'Conecte ao Google Drive para ver as cópias.'));
  const avisoConexao = st.erro ? el('p', { class: 'ajuda' }, `Não foi possível conectar: ${st.erro}`) : null;

  const carregarLista = async () => {
    try {
      const itens = await invocar('nuvem_listar');
      const linhas = itens.map((a) => ({
        ...a,
        quando: new Date(a.criado_em).toLocaleString('pt-BR'),
        acao: el('button', { type: 'button', class: 'btn secundario', onclick: () => baixarCopia(a) }, 'Baixar cópia'),
      }));
      lista.replaceChildren(tabela([
        { key: 'nome', label: 'Arquivo' },
        { key: 'quando', label: 'Criado em' },
        { key: 'acao', label: '', render: (a) => a.acao },
      ], linhas, 'Nenhuma cópia no Google Drive ainda.'));
    } catch (e) {
      lista.replaceChildren(el('p', { class: 'ajuda' }, e.texto));
    }
  };

  const baixarCopia = async (a) => {
    try {
      const caminho = await invocar('nuvem_baixar', { id: a.id });
      aviso(`Cópia salva em: ${caminho}. O sistema NÃO foi trocado. Para restaurar, feche o sistema, guarde o banco atual e só então substitua-o pela cópia baixada.`);
    } catch (e) { aviso(e.texto, 'erro'); }
  };

  // Espera a autorização no navegador (o servidor local aguarda até 5 minutos).
  const acompanharConexao = async () => {
    for (let i = 0; i < 150; i++) {
      await new Promise((r) => setTimeout(r, 2000));
      try {
        const atual = await invocar('nuvem_status');
        if (atual.conexao === 'aguardando') continue;
        if (atual.conexao === 'erro') { estado.textContent = 'Não conectado.'; aviso(`Não foi possível conectar: ${atual.erro}`, 'erro'); return; }
        estado.textContent = textoEstadoNuvem(atual);
        aviso('Conta do Google Drive conectada.');
        carregarLista();
        return;
      } catch (e) { aviso(e.texto, 'erro'); return; }
    }
    estado.textContent = 'O tempo para autorizar acabou. Tente conectar de novo.';
  };

  const bloco = el('div', { class: 'nuvem' },
    el('details', { class: 'nuvem-passos' },
      el('summary', {}, 'Como criar o projeto no Google Cloud (faça uma única vez)'),
      el('ol', {}, PASSOS_GOOGLE.map((p) => el('li', {}, p)))),
    el('label', { class: 'campo' }, el('span', { class: 'rotulo' }, 'Client ID'), idInput),
    el('label', { class: 'campo' }, el('span', { class: 'rotulo' }, 'Client Secret'), segredoInput),
    el('label', { class: 'campo' }, el('span', { class: 'rotulo' }, 'Manter as últimas cópias no Google Drive'), manterInput,
      el('small', { class: 'ajuda' }, `De 1 a 100. Padrão: ${st.padrao ?? 10}. Só as cópias antigas criadas por este sistema são apagadas.`)),
    botao({ texto: 'Salvar dados do Google', icone: 'disco', onclick: async () => {
      try {
        await invocar('nuvem_configurar', { clientId: idInput.value, clientSecret: segredoInput.value, manter: Number(manterInput.value) });
        aviso('Dados salvos.');
        reabrirPagina();
      } catch (e) { aviso(e.texto, 'erro'); }
    } }),
    el('p', { class: 'ajuda' }, 'Ao conectar, o navegador abre para você autorizar a conta Google da oficina. O sistema só acessa os arquivos de backup que ele mesmo cria.'),
    estado,
    avisoConexao,
    el('div', { class: 'acoes' },
      botao({ texto: 'Conectar ao Google Drive', icone: 'nuvem', tipo: 'primario', onclick: async () => {
        try {
          const r = await invocar('nuvem_conectar');
          estado.textContent = 'Aguardando a autorização no navegador...';
          if (!r.navegador_aberto) estado.append(el('br'), `Não abriu sozinho. Copie este endereço no navegador: ${r.url}`);
          acompanharConexao();
        } catch (e) { aviso(e.texto, 'erro'); }
      } }),
      botao({ texto: 'Desconectar', icone: 'alerta', onclick: async () => {
        try { await invocar('nuvem_desconectar'); estado.textContent = 'Não conectado.'; lista.replaceChildren(el('p', { class: 'ajuda' }, 'Conecte ao Google Drive para ver as cópias.')); aviso('Conta desconectada.'); }
        catch (e) { aviso(e.texto, 'erro'); }
      } }),
      botao({ texto: 'Enviar backup para o Google Drive agora', icone: 'nuvem', tipo: 'primario', onclick: async () => {
        try {
          const r = await invocar('nuvem_enviar');
          aviso(`Cópia enviada: ${r.nome}. Cópias antigas removidas: ${r.antigas_removidas}.`);
          carregarLista();
        } catch (e) { aviso(e.texto, 'erro'); }
      } })),
    el('h4', {}, 'Cópias no Google Drive'),
    el('div', { class: 'acoes' }, botao({ texto: 'Atualizar lista', icone: 'setas', onclick: carregarLista })),
    lista,
    el('p', { class: 'ajuda' }, 'Restaurar: baixar uma cópia não troca o banco do sistema. Isso é feito manualmente, com o sistema fechado.'),
  );

  if (st.conectado) carregarLista();
  return secao('Backup no Google Drive', 'nuvem', bloco);
}

// ---------- Cópia e restauração do banco (administrador) ----------
// Cópia: arquivo .db guardado neste computador. Restauração: escolher um .db, conferir e confirmar digitando RESTAURAR.
async function secaoCopiasLocais() {
  const c = await invocar('config_obter');
  const ultimo = el('p', { class: 'texto-grande' }, `Último backup: ${c.ultimo_backup ? new Date(c.ultimo_backup).toLocaleString('pt-BR') : 'nenhuma cópia feita ainda'}`);
  const resumo = el('div', { class: 'achados', 'aria-live': 'polite' });
  const entrada = el('input', { type: 'file', accept: '.db', 'aria-label': 'Escolher o arquivo de backup (.db)' });
  let arquivo = null;

  function mostrarResumo(r) {
    arquivo = r;
    const confirmacao = el('input', { type: 'text', autocomplete: 'off', placeholder: 'Digite RESTAURAR', 'aria-label': 'Digite RESTAURAR para confirmar' });
    resumo.replaceChildren(
      el('h4', {}, 'Conferir antes de restaurar'),
      el('ul', { class: 'lista-simples' },
        el('li', {}, `Arquivo: ${r.nome_arquivo}`),
        el('li', {}, `Data do arquivo: ${r.data_arquivo || 'não disponível'}`),
        el('li', {}, `Clientes: ${r.clientes}`),
        el('li', {}, `Veículos: ${r.veiculos}`),
        el('li', {}, `Ordens de serviço: ${r.ordens}`),
        el('li', {}, `Funcionários: ${r.funcionarios}`)),
      el('p', { class: 'faixa erro' }, icone('alerta', 24),
        el('strong', {}, 'A restauração substitui TODOS os dados atuais deste computador.')),
      el('p', { class: 'ajuda' }, 'Antes de substituir, o sistema guarda uma cópia dos dados atuais na pasta de backups.'),
      el('label', { class: 'campo' }, el('span', { class: 'rotulo' }, 'Para confirmar, digite RESTAURAR'), confirmacao),
      botao({ texto: 'Restaurar este backup', icone: 'alerta', tipo: 'perigo', onclick: async () => {
        if (confirmacao.value.trim() !== 'RESTAURAR') { aviso('Para confirmar, digite a palavra RESTAURAR.', 'erro'); return; }
        try {
          await invocar('backup_restaurar', { caminho: arquivo.caminho, confirmacao: confirmacao.value.trim() });
          aviso('Dados restaurados com sucesso. A tela vai recarregar e pedir o login de novo.');
          setTimeout(() => window.location.reload(), 3000);
        } catch (e) { aviso(e.texto, 'erro'); }
      } }));
  }

  entrada.addEventListener('change', async () => {
    const f = entrada.files?.[0];
    entrada.value = '';
    if (!f) return;
    try {
      const dados = await new Promise((ok, falha) => {
        const leitor = new FileReader();
        leitor.onload = () => ok(String(leitor.result).split(',')[1]);
        leitor.onerror = () => falha(new Error('leitura'));
        leitor.readAsDataURL(f);
      });
      mostrarResumo(await invocar('backup_validar_arquivo', { dados }));
    } catch (e) {
      resumo.replaceChildren();
      aviso(e.texto ?? 'Não foi possível ler este arquivo.', 'erro');
    }
  });

  return secao('Cópia e restauração do banco de dados', 'disco',
    ultimo,
    el('p', { class: 'ajuda' }, 'As cópias ficam guardadas neste computador, na pasta de backups do sistema. Uma cópia automática é feita a cada 6 horas.'),
    el('div', { class: 'acoes' }, botao({ texto: 'Criar cópia agora', icone: 'disco', tipo: 'primario', onclick: async () => {
      try {
        await invocar('backup_agora');
        aviso('Cópia criada com sucesso.');
        reabrirPagina();
      } catch (e) { aviso(e.texto, 'erro'); }
    } })),
    el('h4', {}, 'Restaurar uma cópia'),
    el('p', { class: 'ajuda' }, 'Escolha um arquivo .db de backup deste sistema. Arquivos de outras versões do sistema são recusados.'),
    el('label', { class: 'campo' }, el('span', { class: 'rotulo' }, 'Escolher arquivo de backup'), entrada),
    resumo);
}
