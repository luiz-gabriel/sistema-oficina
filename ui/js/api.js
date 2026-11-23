// Ponte com o backend Tauri. Operações de negócio passam por `comando` (sessão, permissão e modo de rede).
const invoke = (...args) => window.__TAURI__.core.invoke(...args);

export async function invocar(comando, args = {}) {
  try {
    return await invoke(comando, args);
  } catch (erro) {
    throw normalizarErro(erro);
  }
}

export const chamar = (nome, args = {}) => invocar('comando', { nome, args });

// Erros chegam como { kind, message }. Em validação, message é a lista de campos.
export function normalizarErro(erro) {
  if (erro && erro.kind === 'validation' && Array.isArray(erro.message)) {
    const campos = Object.fromEntries(erro.message.map((e) => [e.field, e.message]));
    return { texto: 'Revise os campos destacados.', campos };
  }
  return { texto: typeof erro?.message === 'string' ? erro.message : String(erro), campos: {} };
}
