/// The page an operator signs on.
///
/// `client_id` and `redirect_uri` are displayed, not merely posted back as
/// hidden fields: the operator is approving delivery of a credential to a
/// specific place and cannot weigh that without seeing where. Both are already
/// constrained by the time this renders — the registration was screened against
/// `mcpRedirectOrigins`, and the URI exact-matched against that registration —
/// so this is the last human check, not the enforcing one.
pub(super) fn login_html(
    fields: &[(String, String)],
    error: Option<&str>,
    authorize_url: &str,
    client_id: &str,
    redirect_uri: &str,
) -> String {
    let error = error
        .map(|e| format!("<p class=\"error\">{}</p>", html(e)))
        .unwrap_or_default();
    let inputs = fields
        .iter()
        .map(|(name, value)| {
            format!(
                r#"<input type="hidden" name="{}" value="{}">"#,
                html(name),
                html(value)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let script = r#"<script>
const form = document.getElementById("login-form");
const button = document.getElementById("nip07-button");
const status = document.getElementById("login-status");
const nsecInput = document.getElementById("nsec-input");

button.addEventListener("click", async () => {
  try {
    if (!window.nostr || !window.nostr.getPublicKey || !window.nostr.signEvent) {
      throw new Error("No NIP-07 signer was found in this browser.");
    }
    button.disabled = true;
    status.textContent = "Waiting for signer approval...";
    const pubkey = await window.nostr.getPublicKey();
    const event = await window.nostr.signEvent({
      kind: 27235,
      created_at: Math.floor(Date.now() / 1000),
      tags: [
        ["u", form.dataset.authorizeUrl],
        ["method", "POST"],
        ["challenge", form.elements.login_challenge.value],
        ["client", "mosaico-mcp"]
      ],
      content: "mosaico OAuth login"
    });
    form.elements.nip07_pubkey.value = pubkey;
    form.elements.nip07_event.value = JSON.stringify(event);
    nsecInput.required = false;
    status.textContent = "Approved. Returning to ChatGPT...";
    form.submit();
  } catch (err) {
    status.textContent = err && err.message ? err.message : String(err);
    button.disabled = false;
  }
});
</script>"#;
    format!(
        r#"<!doctype html>
<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">
<title>mosaico login</title>
<style>
:root{{color-scheme:light;--paper:#f7f4ee;--ink:#13150f;--muted:#6d6c64;--teal:#1c6b64;--ease:cubic-bezier(.22,1,.36,1)}}
*{{box-sizing:border-box}}
html{{min-height:100%;background:var(--paper)}}
body{{min-height:100dvh;margin:0;color:var(--ink);font-family:"Geist","Plus Jakarta Sans",ui-sans-serif,sans-serif;background:linear-gradient(135deg,#fbfaf6 0%,#eef0ea 45%,#f4ece4 100%)}}
body::before{{content:"";position:fixed;inset:0;pointer-events:none;opacity:.26;background-image:linear-gradient(rgba(19,21,15,.035) 1px,transparent 1px),linear-gradient(90deg,rgba(19,21,15,.03) 1px,transparent 1px);background-size:42px 42px}}
main{{min-height:100dvh;width:100%;padding:clamp(2rem,6vw,5rem);display:grid;grid-template-columns:minmax(0,1fr) minmax(23rem,30rem);gap:clamp(2rem,6vw,7rem);align-items:center}}
.copy,.shell{{animation:rise 900ms var(--ease) both}}
.shell{{animation-delay:120ms;padding:.48rem;border-radius:2rem;background:rgba(255,255,255,.42);box-shadow:inset 0 0 0 1px rgba(255,255,255,.7),0 38px 90px rgba(51,47,38,.14)}}
.eyebrow{{display:inline-flex;align-items:center;gap:.55rem;border-radius:999px;padding:.45rem .75rem;color:#41453a;background:rgba(255,255,255,.56);box-shadow:inset 0 0 0 1px rgba(19,21,15,.08);font-size:.64rem;font-weight:700;letter-spacing:.18em;text-transform:uppercase}}
.dot{{width:.42rem;height:.42rem;border-radius:999px;background:var(--teal);box-shadow:0 0 0 .35rem rgba(28,107,100,.1)}}
h1{{max-width:10ch;margin:1.45rem 0 1.1rem;font-family:"Clash Display","Geist",ui-sans-serif,sans-serif;font-size:clamp(3.9rem,8vw,7.8rem);line-height:.86;letter-spacing:0}}
.lede{{max-width:34rem;margin:0;color:var(--muted);font-size:clamp(1.05rem,1.7vw,1.22rem);line-height:1.72}}
.proofs{{display:flex;flex-wrap:wrap;gap:.75rem;margin-top:2rem}}
.proof{{border-radius:999px;padding:.65rem .85rem;background:rgba(255,255,255,.48);color:#484b42;box-shadow:inset 0 0 0 1px rgba(19,21,15,.075);font-size:.78rem;font-weight:650}}
.panel{{position:relative;overflow:hidden;border-radius:calc(2rem - .48rem);padding:clamp(1.35rem,3vw,1.85rem);background:linear-gradient(180deg,rgba(255,255,255,.96),rgba(252,250,245,.88));box-shadow:inset 0 1px 0 rgba(255,255,255,.9),inset 0 0 0 1px rgba(19,21,15,.08)}}
.panel::before{{content:"";position:absolute;inset:0;pointer-events:none;opacity:.8;background:linear-gradient(140deg,rgba(216,182,161,.24),transparent 32%,rgba(28,107,100,.1))}}
.panel>*{{position:relative}}
.brand-row{{display:flex;align-items:center;justify-content:space-between;gap:1rem;margin-bottom:1.2rem}}
.mark{{width:3.05rem;height:3.05rem;border-radius:1.05rem;display:grid;place-items:center;background:#141710;color:#f8f3ea;font-family:"Clash Display","Geist",ui-sans-serif,sans-serif;font-size:1.05rem;box-shadow:inset 0 1px 0 rgba(255,255,255,.16)}}
.state-pill{{border-radius:999px;padding:.45rem .68rem;background:rgba(143,154,107,.14);color:#535b37;font-size:.68rem;font-weight:750;letter-spacing:.12em;text-transform:uppercase}}
h2{{margin:0 0 .45rem;font-size:1.72rem;line-height:1.05;letter-spacing:0}}
.micro{{margin:0 0 1.35rem;color:#747167;font-size:.78rem;line-height:1.55}}
.hidden-field{{display:none}}
.destination{{margin:0 0 1.2rem;padding:.85rem .9rem;border-radius:1rem;background:rgba(19,21,15,.045);box-shadow:inset 0 0 0 1px rgba(19,21,15,.065)}}
.destination dt{{color:#65665c;font-size:.66rem;font-weight:750;letter-spacing:.12em;text-transform:uppercase}}
.destination dd{{margin:.2rem 0 .7rem;color:#23261e;font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:.78rem;line-height:1.4;overflow-wrap:anywhere}}
.destination dd:last-child{{margin-bottom:0}}
.primary,.secondary{{width:100%;border:0;appearance:none;cursor:pointer;border-radius:999px;padding:.55rem .55rem .55rem 1.18rem;display:flex;align-items:center;justify-content:space-between;gap:1rem;font:750 .95rem/1 "Geist","Plus Jakarta Sans",ui-sans-serif,sans-serif;transition:transform 700ms var(--ease),opacity 700ms var(--ease)}}
.primary{{background:#141710;color:#fff7ee;box-shadow:inset 0 1px 0 rgba(255,255,255,.18)}}
.secondary{{margin-top:.8rem;background:rgba(19,21,15,.06);color:#23261e;box-shadow:inset 0 0 0 1px rgba(19,21,15,.08)}}
.primary:hover,.secondary:hover{{transform:translateY(-2px)}}
.primary:active,.secondary:active{{transform:scale(.982)}}
.primary:disabled{{opacity:.58;cursor:wait}}
.icon{{width:2.45rem;height:2.45rem;border-radius:999px;display:grid;place-items:center;background:rgba(255,255,255,.12);transition:transform 700ms var(--ease),opacity 700ms var(--ease)}}
.secondary .icon{{background:rgba(19,21,15,.07)}}
.primary:hover .icon,.secondary:hover .icon{{transform:translate3d(.18rem,-.05rem,0) scale(1.04)}}
.status{{min-height:1.2rem;margin:.85rem 0 0;color:#555a4d;font-size:.82rem;line-height:1.45}}
.error{{margin:0 0 1rem;border-radius:1rem;padding:.8rem .9rem;background:rgba(142,38,30,.08);color:#8e261e;box-shadow:inset 0 0 0 1px rgba(142,38,30,.14);font-size:.84rem}}
.fallback{{margin-top:1.35rem;padding-top:1.25rem;box-shadow:inset 0 1px 0 rgba(19,21,15,.08)}}
label{{display:block;color:#65665c;font-size:.75rem;font-weight:750;letter-spacing:.12em;text-transform:uppercase}}
.field-shell{{margin-top:.55rem;padding:.28rem;border-radius:1.15rem;background:rgba(19,21,15,.045);box-shadow:inset 0 0 0 1px rgba(19,21,15,.065)}}
input[type=password]{{width:100%;border:0;outline:0;border-radius:.9rem;padding:.9rem .95rem;color:var(--ink);background:rgba(255,255,255,.86);box-shadow:inset 0 1px 0 rgba(255,255,255,.7);font:600 .94rem/1 "Geist","Plus Jakarta Sans",ui-sans-serif,sans-serif}}
input[type=password]:focus{{box-shadow:inset 0 0 0 1px rgba(28,107,100,.35),0 0 0 .25rem rgba(28,107,100,.08)}}
@keyframes rise{{from{{opacity:0;transform:translate3d(0,2.2rem,0)}}to{{opacity:1;transform:translate3d(0,0,0)}}}}
@media (max-width:767px){{main{{grid-template-columns:1fr;padding:2rem 1rem;gap:2rem;align-items:start}}h1{{font-size:clamp(3.25rem,18vw,5.2rem)}}.lede{{font-size:1rem}}.proofs{{margin-top:1.4rem}}}}
@media (prefers-reduced-motion:reduce){{.copy,.shell{{animation:none}}.primary,.secondary,.icon{{transition:none}}}}
</style>
<main>
  <section class="copy" aria-label="mosaico OAuth pairing">
    <span class="eyebrow"><span class="dot"></span>OAuth Pairing</span>
    <h1>mosaico</h1>
    <p class="lede">Authorize ChatGPT to reach your agent fabric with a whitelisted Nostr identity. The signer proves control; the server keeps the session token scoped.</p>
    <div class="proofs" aria-label="Security notes">
      <span class="proof">NIP-07 first</span>
      <span class="proof">one-time challenge</span>
      <span class="proof">whitelist gated</span>
    </div>
  </section>
  <section class="shell" aria-label="Pairing form">
    <form class="panel" id="login-form" method="post" action="/oauth/authorize" data-authorize-url="{authorize_url}">
      {inputs}
      <input class="hidden-field" name="nip07_pubkey" type="hidden">
      <input class="hidden-field" name="nip07_event" type="hidden">
      <div class="brand-row">
        <div class="mark">te</div>
        <div class="state-pill">secure handoff</div>
      </div>
      <h2>Pair this session</h2>
      <p class="micro">Use a browser Nostr signer, or fall back to a whitelisted nsec when an extension is not available.</p>
      <dl class="destination">
        <dt>Registered client</dt><dd>{client_id}</dd>
        <dt>Delivers the code to</dt><dd>{redirect_uri}</dd>
      </dl>
      {error}
      <button class="primary" id="nip07-button" type="button"><span>Pair with NIP-07</span><span class="icon" aria-hidden="true">↗</span></button>
      <p id="login-status" class="status" aria-live="polite"></p>
      <div class="fallback">
        <label for="nsec-input">Fallback nsec</label>
        <div class="field-shell"><input id="nsec-input" name="nsec" type="password" autocomplete="off" required></div>
        <button class="secondary" type="submit"><span>Pair with nsec</span><span class="icon" aria-hidden="true">→</span></button>
      </div>
    </form>
  </section>
</main>
{script}"#,
        authorize_url = html(authorize_url),
        inputs = inputs,
        client_id = html(client_id),
        redirect_uri = html(redirect_uri),
        error = error,
        script = script,
    )
}

fn html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
