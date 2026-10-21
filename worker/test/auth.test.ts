import { afterEach, describe, expect, it, vi } from 'vitest';
import { exportJWK, generateKeyPair, SignJWT } from 'jose';
import { verifyToken } from '../src/auth';

afterEach(() => vi.unstubAllGlobals());

describe('Worker JWT verification', () => {
  it('verifies an asymmetric token locally after one JWKS fetch', async () => {
    const base = 'https://auth-test.supabase.co';
    const { publicKey, privateKey } = await generateKeyPair('ES256');
    const jwk = { ...await exportJWK(publicKey), kid: 'current', alg: 'ES256', use: 'sig' };
    const token = await new SignJWT({ role: 'authenticated' })
      .setProtectedHeader({ alg: 'ES256', kid: 'current' })
      .setIssuer(`${base}/auth/v1`).setAudience('authenticated')
      .setSubject('user-1').setExpirationTime('5m').sign(privateKey);
    const fetchMock = vi.fn(async (_input: unknown) => Response.json({ keys: [jwk] }));
    vi.stubGlobal('fetch', fetchMock);
    const request = new Request('https://worker.test/', { headers: { Authorization: `Bearer ${token}` } });
    const env = { SUPABASE_URL: base, SUPABASE_ANON_KEY: 'public' };

    expect(await verifyToken(request, env)).toEqual({ id: 'user-1' });
    expect(await verifyToken(request, env)).toEqual({ id: 'user-1' });
    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(fetchMock.mock.calls[0]?.[0]).toBe(`${base}/auth/v1/.well-known/jwks.json`);
  });

  it('keeps the Auth-server check for legacy HS256 tokens', async () => {
    const header = btoa(JSON.stringify({ alg: 'HS256', typ: 'JWT' }));
    const fetchMock = vi.fn(async () => Response.json({ id: 'legacy-user' }));
    vi.stubGlobal('fetch', fetchMock);
    const request = new Request('https://worker.test/', {
      headers: { Authorization: `Bearer ${header}.payload.signature` },
    });
    expect(await verifyToken(request, {
      SUPABASE_URL: 'https://legacy.supabase.co', SUPABASE_ANON_KEY: 'public',
    })).toEqual({ id: 'legacy-user' });
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });
});
