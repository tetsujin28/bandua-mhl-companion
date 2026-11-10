"""Captures d'écran de Bandua MHL Companion en mode démo, dans les trois langues.

    pip install playwright pillow && playwright install chromium
    python docs/tools/captures.py          # fr en es
    python docs/tools/annoter.py           # cadres et zooms dans docs/guide/

Les captures brutes et la position des éléments sont écrites dans docs/tools/out/.
Variable CHROME : chemin d'un Chrome installé (sinon le Chromium de Playwright).
"""
import sys, os, json, asyncio, pathlib
from playwright.async_api import async_playwright
ROOT = pathlib.Path(__file__).resolve().parents[2]
OUT = ROOT / "docs" / "tools" / "out"
URL = (ROOT / "ui" / "index.html").as_uri()
CHROME = os.environ.get("CHROME")
SCALE = 1.5
SEL = {"zone": "#zone", "pickFolder": "#pickFolder", "pickMhl": "#pickMhl", "dropNote": "#zone .note",
       "src": ".left > div:nth-child(1)", "changeBtn": "#changeBtn", "pkgs": ".left > div:nth-child(2)", "pk0": "#pk0", "pk1": "#pk1",
       "ring": ".ring", "metrics": ".metrics", "cur": "#cur", "bar": ".track", "dash": ".dash", "verdict": "#verdict .big",
       "verdictBox": "#verdict", "tabs": "#tabs", "tabProblems": "#tabs [data-tab=problems]", "tabClips": "#tabs [data-tab=clips]",
       "tabExtra": "#tabs [data-tab=extra]", "pane": "#pane", "issue0": "#pane .issue:first-child", "reveal0": "#pane [data-reveal]",
       "badRow": "#pane tr:has(.bad), #pane tr.bad", "go": "#go", "exportBtn": "#exportBtn", "newBtn": "#newBtn", "footMsg": "#footMsg",
       "lang": "header #lang", "aboutBtn": "#aboutBtn", "chip": "#chip", "toast": "#toast .toast", "modalBox": ".modal .box",
       "warnb": ".warnb", "closeAbout": "#closeAbout"}
OK_ONLY = """(() => { const o = onEvent; onEvent = (e) => {
  if (e.type === 'issue') return;
  if (e.type === 'package_done') { const r = e.result; r.issues = []; r.extra = []; r.extra_total = 0; r.ok_files = r.checked; r.clips.forEach(c => { c.status = 'ok'; c.bad = 0; }); }
  if (e.type === 'finished') { e.report.ok = true; e.report.problems = 0; e.report.extra_total = 0; }
  o(e); }; })()"""

async def shot(page, name):
    await page.wait_for_timeout(1200)
    await page.screenshot(path=str(OUT / f"{name}.png"))
    r = {}
    for k, q in SEL.items():
        b = await page.evaluate("q=>{const e=document.querySelector(q);if(!e)return null;const r=e.getBoundingClientRect();if(!r.width)return null;return [r.left,r.top,r.right,r.bottom]}", q)
        if b: r[k] = [round(v * SCALE) for v in b]
    t = await page.evaluate("()=>{const e=document.getElementById('footMsg');if(!e.firstChild)return null;const g=document.createRange();g.selectNodeContents(e);const r=g.getBoundingClientRect();return [r.left,r.top,r.right,r.bottom]}")
    if t: r["footText"] = [round(v * SCALE) for v in t]
    json.dump(r, open(OUT / f"{name}.json", "w"))

async def fresh(b, lang, ack=True):
    ctx = await b.new_context(viewport={"width": 1180, "height": 780}, device_scale_factor=SCALE, locale=lang)
    page = await ctx.new_page()
    await page.clock.install()
    init = f"localStorage.setItem('bmc_lang','{lang}');"
    if ack: init += "localStorage.setItem('bmc_ack','1');"
    await page.add_init_script(init)
    await page.goto(URL)
    await page.add_style_tag(content="#demoTag{display:none!important}")
    return page

async def until(page, js, step=60, limit=4000):
    for _ in range(limit):
        await page.clock.run_for(step)
        if await page.evaluate(js): return

async def main(lang):
    async with async_playwright() as p:
        b = await (p.chromium.launch(executable_path=CHROME) if CHROME else p.chromium.launch())
        page = await fresh(b, lang, ack=False); await page.clock.run_for(300); await shot(page, f"welcome_{lang}")
        await page.click("#closeAbout"); await page.clock.run_for(200); await shot(page, f"empty_{lang}")
        await page.click("#pickFolder"); await page.clock.run_for(1000); await shot(page, f"ready_{lang}")
        await page.click("#go"); await page.clock.run_for(4300); await shot(page, f"running_{lang}")
        await until(page, "S.phase==='done'"); await page.clock.run_for(300); await shot(page, f"problems_{lang}")
        await page.click("#tabs [data-tab=clips]"); await page.clock.run_for(200); await shot(page, f"clips_{lang}")
        await page.click("#tabs [data-tab=extra]"); await page.clock.run_for(200); await shot(page, f"extra_{lang}")
        await page.click("#tabs [data-tab=problems]"); await page.click("#exportBtn"); await page.clock.run_for(300); await shot(page, f"export_{lang}")
        page = await fresh(b, lang); await page.clock.run_for(300); await page.evaluate(OK_ONLY)
        await page.click("#pickFolder"); await page.clock.run_for(1000); await page.click("#go")
        await until(page, "S.phase==='done'"); await page.clock.run_for(300); await shot(page, f"ok_{lang}")
        await b.close()

OUT.mkdir(exist_ok=True)
for l in (sys.argv[1:] or ["fr", "en", "es"]): asyncio.run(main(l))
