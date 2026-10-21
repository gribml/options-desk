import { createRemoteJWKSet, decodeProtectedHeader, jwtVerify } from 'jose';

interface AuthEnv {
  SUPABASE_URL: string;
  SUPABASE_ANON_KEY: string;
}

export interface AuthUser { id: string }

const jwksByIssuer = new Map<string, ReturnType<typeof createRemoteJWKSet>>();

// Supabase's asymmetric keys allow local verification after JWKS discovery.
// Legacy HS256 projects must continue using the Auth server for verification.
export async function verifyToken(request: Request, env: AuthEnv): Promise<AuthUser | null> {
  const authorization = request.headers.get('Authorization');
  if (!authorization?.startsWith('Bearer ')) return null;
  const token = authorization.slice(7);
  const base = env.SUPABASE_URL.replace(/\/rest\/v1\/?$/, '').replace(/\/$/, '');
  try {
    const { alg } = decodeProtectedHeader(token);
    if (alg !== 'HS256') {
      const issuer = `${base}/auth/v1`;
      let jwks = jwksByIssuer.get(issuer);
      if (!jwks) {
        jwks = createRemoteJWKSet(new URL(`${issuer}/.well-known/jwks.json`), { cooldownDuration: 30_000 });
        jwksByIssuer.set(issuer, jwks);
      }
      const { payload } = await jwtVerify(token, jwks, { issuer, audience: 'authenticated' });
      return typeof payload.sub === 'string' ? { id: payload.sub } : null;
    }
  } catch {
    return null;
  }

  try {
    const response = await fetch(`${base}/auth/v1/user`, {
      headers: { apikey: env.SUPABASE_ANON_KEY, Authorization: authorization },
    });
    if (!response.ok) return null;
    const user = await response.json<{ id?: unknown }>();
    return typeof user.id === 'string' ? { id: user.id } : null;
  } catch {
    return null;
  }
}
