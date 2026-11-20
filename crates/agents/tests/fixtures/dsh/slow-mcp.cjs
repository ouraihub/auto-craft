// Real stdio MCP server used only by the ignored DSH long-tool probe.
const readline = require('node:readline');
const fs = require('node:fs');
const log = (event, extra = {}) => fs.appendFileSync(process.argv[2], JSON.stringify({event, at_ms: Date.now(), ...extra}) + '\n');
const send = frame => {
  const response = {jsonrpc: '2.0', ...frame};
  log('send', {frame: response});
  process.stdout.write(JSON.stringify(response) + '\n');
};
readline.createInterface({input: process.stdin}).on('line', line => {
  const frame = JSON.parse(line);
  log('receive', {frame});
  if (frame.method === 'initialize') {
    send({id: frame.id, result: {protocolVersion: frame.params.protocolVersion, capabilities: {tools: {}}, serverInfo: {name: 'autocraft-slow-probe', version: '1'}}});
  } else if (frame.method === 'tools/list') {
    send({id: frame.id, result: {tools: [{name: 'slow', description: 'Wait seventy seconds; report progress only when the client supplies a progress token.', inputSchema: {type: 'object', properties: {}, additionalProperties: false}}]}});
  } else if (frame.method === 'tools/call') {
    const token = frame.params._meta?.progressToken;
    const tokenPresent = typeof token === 'string' || (typeof token === 'number' && Number.isFinite(token));
    log('call', {request_id: frame.id, params: frame.params, progress_token_present: tokenPresent});
    if (!tokenPresent) log('progress_unavailable', {request_id: frame.id, reason: token === undefined ? 'missing_progress_token' : 'invalid_progress_token'});
    let elapsed = 0;
    const interval = setInterval(() => {
      elapsed += 5;
      if (tokenPresent) send({method: 'notifications/progress', params: {progressToken: token, progress: elapsed, total: 70}});
      if (elapsed === 70) {
        clearInterval(interval);
        send({id: frame.id, result: {content: [{type: 'text', text: 'AUTOCRAFT_SLOW_COMPLETED'}], isError: false}});
      }
    }, 5000);
  } else if (frame.method === 'notifications/cancelled') {
    log('cancelled', {params: frame.params});
  } else if (frame.id !== undefined) {
    send({id: frame.id, error: {code: -32601, message: 'Unknown method'}});
  }
});
process.stdin.on('end', () => process.exit(0));
