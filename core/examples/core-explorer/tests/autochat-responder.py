"""SYNTHETIC deterministic responder; no model, network, credentials, or SDK."""
import json
import sys

value = json.load(sys.stdin)
if set(value) != {'text'} or not isinstance(value['text'], str):
    raise SystemExit(2)
print(json.dumps({'text': 'Synthetic automatic reply: ' + value['text'][:120]}))
