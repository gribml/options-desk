// Dev-only access gate.
//
// Allows requests only from a configured IP allowlist; everyone else gets a
// 404, so the site appears not to exist (no banner, no login page, nothing to
// probe). Runs on every request (see dist/_routes.json) — static landing page,
// SPA assets, and /app/* routes alike.
//
// Configure the allowlist as a Pages environment variable / secret named
// ALLOWED_IPS: a comma-separated list of client IPs, e.g.
//   "203.0.113.7, 2001:db8::1234"
// Find your current public IP with:  curl https://api.ipify.org   (IPv4)
//                                    curl https://api64.ipify.org  (IPv6, if any)
//
// Fail-closed: if ALLOWED_IPS is unset or empty, ALL requests get 404.
//
// To lift the lockdown: delete this file and dist/_routes.json (and drop the
// `cp landing/_routes.json` line from the deploy workflow), then redeploy.
export async function onRequest(ctx) {
  const allow = (ctx.env.ALLOWED_IPS || "")
    .split(",")
    .map((s) => s.trim())
    .filter(Boolean);

  const ip = ctx.request.headers.get("CF-Connecting-IP") || "";

  if (allow.length === 0 || !allow.includes(ip)) {
    return new Response("Not Found", {
      status: 404,
      headers: { "content-type": "text/plain; charset=utf-8" },
    });
  }

  // Allowed: fall through to the static-asset pipeline (incl. _redirects).
  return ctx.next();
}
