// Dev-only access gate.
//
// Lets a request through if EITHER
//   - it carries a valid Cloudflare Access token (Cf-Access-Jwt-Assertion), or
//   - its client IP is on the ALLOWED_IPS list.
// Everyone else gets a 404, so the site appears not to exist (no banner, no
// login page, nothing to probe). Runs on every request (see dist/_routes.json)
// and on every hostname the Pages project answers on — martingale.cc, the
// <project>.pages.dev alias, and preview deployments alike. A hostname that
// isn't behind an Access application never gets a token, so it 404s unless the
// client IP is allowlisted.
//
// Pages environment variables / secrets:
//   ACCESS_TEAM_DOMAIN  Zero Trust team domain, e.g. "yourteam.cloudflareaccess.com"
//   ACCESS_AUD          Application Audience (AUD) tag of the Access application.
//                       Comma-separate several to accept more than one app
//                       (e.g. a separate one covering preview deployments).
//   ALLOWED_IPS         Optional comma-separated client IPs that skip the login,
//                       e.g. "203.0.113.7, 2001:db8::1234". Find your public IP
//                       with  curl https://api.ipify.org  /  curl https://api64.ipify.org
//
// Access "Bypass" policies don't issue a token, so an IP that should skip the
// login belongs in ALLOWED_IPS, not in a Bypass rule.
//
// Fail-closed: with ALLOWED_IPS unset and either Access variable unset, ALL
// requests get 404.
//
// To lift the lockdown: delete this file and landing/_routes.json (and drop the
// `cp landing/_routes.json dist/_routes.json` line from the deploy workflow),
// then redeploy.
export async function onRequest(ctx) {
  const allowedIps = list(ctx.env.ALLOWED_IPS);
  const ip = ctx.request.headers.get("CF-Connecting-IP") || "";
  if (ip && allowedIps.includes(ip)) return ctx.next();

  const team = (ctx.env.ACCESS_TEAM_DOMAIN || "")
    .trim()
    .replace(/^https?:\/\//, "")
    .replace(/\/+$/, "");
  const auds = list(ctx.env.ACCESS_AUD);
  const token = ctx.request.headers.get("Cf-Access-Jwt-Assertion");
  if (team && auds.length && token && (await verifyAccessJwt(token, team, auds))) {
    return ctx.next();
  }

  return new Response("Not Found", {
    status: 404,
    headers: { "content-type": "text/plain; charset=utf-8" },
  });
}

function list(value) {
  return (value || "")
    .split(",")
    .map((s) => s.trim())
    .filter(Boolean);
}

// ── Access JWT verification (RS256 against the team's published keys) ─────────

const CERTS_TTL_MS = 10 * 60 * 1000;
// A token naming an unknown key id forces a refetch (Access rotates keys), but
// at most this often, so junk tokens can't turn every request into a fetch.
const CERTS_MIN_REFETCH_MS = 60 * 1000;

// Per-isolate cache: { team, keys: Map<kid, CryptoKey>, fetchedAt }
let certs = null;

async function signingKey(team, kid) {
  const now = Date.now();
  const fresh = certs && certs.team === team && now - certs.fetchedAt < CERTS_TTL_MS;
  if (fresh && certs.keys.has(kid)) return certs.keys.get(kid);
  if (fresh && now - certs.fetchedAt < CERTS_MIN_REFETCH_MS) return null;

  const res = await fetch(`https://${team}/cdn-cgi/access/certs`);
  if (!res.ok) return null;
  const { keys = [] } = await res.json();
  const imported = new Map();
  for (const jwk of keys) {
    if (jwk.kty !== "RSA" || !jwk.kid) continue;
    imported.set(
      jwk.kid,
      await crypto.subtle.importKey(
        "jwk",
        jwk,
        { name: "RSASSA-PKCS1-v1_5", hash: "SHA-256" },
        false,
        ["verify"],
      ),
    );
  }
  certs = { team, keys: imported, fetchedAt: now };
  return imported.get(kid) || null;
}

async function verifyAccessJwt(token, team, auds) {
  try {
    const parts = token.split(".");
    if (parts.length !== 3) return false;
    const [h, p, s] = parts;

    const header = JSON.parse(decodeUtf8(b64url(h)));
    if (header.alg !== "RS256" || !header.kid) return false;

    const key = await signingKey(team, header.kid);
    if (!key) return false;
    const ok = await crypto.subtle.verify(
      "RSASSA-PKCS1-v1_5",
      key,
      b64url(s),
      new TextEncoder().encode(`${h}.${p}`),
    );
    if (!ok) return false;

    const claims = JSON.parse(decodeUtf8(b64url(p)));
    const now = Math.floor(Date.now() / 1000);
    if (claims.iss !== `https://${team}`) return false;
    if (typeof claims.exp !== "number" || claims.exp <= now) return false;
    if (typeof claims.nbf === "number" && claims.nbf > now + 60) return false;
    const tokenAuds = Array.isArray(claims.aud) ? claims.aud : [claims.aud];
    return tokenAuds.some((a) => auds.includes(a));
  } catch {
    return false;
  }
}

function b64url(s) {
  const b64 = s.replace(/-/g, "+").replace(/_/g, "/");
  const padded = b64 + "=".repeat((4 - (b64.length % 4)) % 4);
  return Uint8Array.from(atob(padded), (c) => c.charCodeAt(0));
}

function decodeUtf8(bytes) {
  return new TextDecoder().decode(bytes);
}
