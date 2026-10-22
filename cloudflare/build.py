"""Bundle the reviewed installers for Cloudflare's dashboard code editor (no secrets)."""
import json
from pathlib import Path
root = Path(__file__).resolve().parent.parent
worker = (root / 'cloudflare/worker.mjs').read_text()
installers = {f'/{name}': (root / name).read_text(encoding='utf-8') for name in ['install.sh', 'install.ps1']}
# Keep the PowerShell UTF-8 BOM for Windows PowerShell 5.1.
worker = worker.replace('export default {', 'const worker = {', 1)
worker += '\nconst INSTALLERS = ' + json.dumps(installers, ensure_ascii=True) + ';\n'
worker += 'export default { fetch(request, env, ctx) { return worker.fetch(request, { ...env, INSTALLERS }, ctx); } };\n'
output = root / 'dist/cloudflare-worker.mjs'
output.parent.mkdir(parents=True, exist_ok=True)
output.write_text(worker)
print(output)
