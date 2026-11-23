// Camada de comunicação da interface. No aplicativo (Tauri) usa a ponte nativa `window.__TAURI__`.
// Num navegador comum da rede local usa HTTP, só com as rotas explícitas de `/api/...` (ver `src-tauri/src/web/rotas.rs`).
// Operações que dependem da máquina (impressora, nuvem, backup, arquivos) não existem no navegador: falham com aviso claro.
const NO_APLICATIVO = typeof window !== 'undefined' && !!window.__TAURI__;

// Leituras expostas pela web: nome do comando -> rota. Quem não estiver aqui não chega ao servidor pelo navegador.
const LEITURAS_WEB = {
  clientes_listar: () => '/api/clientes',
  veiculos_listar: () => '/api/veiculos',
  itens_listar: () => '/api/itens',
  ordens_listar: () => '/api/ordens',
  ordens_buscar: (a) => `/api/ordens/${encodeURIComponent(a.id)}`,
  fornecedores_listar: () => '/api/fornecedores',
  funcionarios_listar: () => '/api/funcionarios',
  caixa_situacao: () => '/api/caixa/situacao',
};

// Administração de aparelhos pelo navegador: só o administrador; o servidor confere a permissão em cada rota.
const ADMIN_WEB = {
  dispositivos_listar: { metodo: 'GET', rota: () => '/api/dispositivos' },
  dispositivos_iniciar_pareamento: { metodo: 'POST', rota: () => '/api/dispositivos/pareamento' },
  dispositivos_revogar: { metodo: 'POST', rota: (a) => `/api/dispositivos/${encodeURIComponent(a.id)}/revogar` },
  pareamento_listar_pedidos: { metodo: 'GET', rota: () => '/api/dispositivos/pedidos' },
  pareamento_aprovar: { metodo: 'POST', rota: (a) => `/api/dispositivos/pedidos/${encodeURIComponent(a.id)}/aprovar` },
  pareamento_recusar: { metodo: 'POST', rota: (a) => `/api/dispositivos/pedidos/${encodeURIComponent(a.id)}/recusar` },
};

const indisponivelNoNavegador = (nome) => ({
  texto: `Esta ação ("${nome}") ainda não está disponível pelo navegador. Use o aplicativo no computador principal.`,
  campos: {},
});

// Chamada HTTP. Erros do servidor chegam como { ok: false, erro: { kind, message } }.
async function http(metodo, caminho, corpo) {
  let resposta;
  try {
    resposta = await fetch(caminho, {
      method: metodo,
      credentials: 'same-origin',
      headers: corpo === undefined ? {} : { 'Content-Type': 'application/json' },
      body: corpo === undefined ? undefined : JSON.stringify(corpo),
    });
  } catch {
    throw { texto: 'Não foi possível falar com o computador do sistema. Verifique a conexão com a rede.', campos: {} };
  }
  let dados = null;
  try {
    dados = await resposta.json();
  } catch {
    dados = null;
  }
  if (!resposta.ok) {
    throw normalizarErro(dados?.erro ?? { message: `Erro ${resposta.status}` });
  }
  return dados;
}

export async function invocar(comando, args = {}) {
  if (NO_APLICATIVO) {
    try {
      return await window.__TAURI__.core.invoke(comando, args);
    } catch (erro) {
      throw normalizarErro(erro);
    }
  }
  switch (comando) {
    case 'estado_sistema':
      return http('GET', '/api/estado');
    case 'login':
      return http('POST', '/api/login', { usuario: args.login, senha: args.senha });
    case 'logout':
      return http('POST', '/api/logout', {});
    default:
      throw indisponivelNoNavegador(comando);
  }
}

export async function chamar(nome, args = {}) {
  if (NO_APLICATIVO) {
    return invocar('comando', { nome, args });
  }
  const admin = ADMIN_WEB[nome];
  if (admin) {
    return http(admin.metodo, admin.rota(args));
  }
  const rota = LEITURAS_WEB[nome];
  if (!rota) {
    throw indisponivelNoNavegador(nome);
  }
  return http('GET', rota(args));
}

// Erros chegam como { kind, message }. Em validação, message é a lista de campos.
export function normalizarErro(erro) {
  if (erro && erro.kind === 'validation' && Array.isArray(erro.message)) {
    const campos = Object.fromEntries(erro.message.map((e) => [e.field, e.message]));
    return { texto: 'Revise os campos destacados.', campos };
  }
  return { texto: typeof erro?.message === 'string' ? erro.message : String(erro), campos: {} };
}
