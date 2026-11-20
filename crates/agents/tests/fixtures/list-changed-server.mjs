// Test-only MCP server. Records requests actually received; emits a real list_changed notification.
import readline from 'node:readline';
import fs from 'node:fs';
const log = process.argv[2];
let enabled = false;
const tool = (name, properties = {}) => ({ name, description: `P05 fixture ${name}`, inputSchema: { type: 'object', properties } });
const send = (value) => process.stdout.write(JSON.stringify(value) + '\n');
for await (const line of readline.createInterface({ input: process.stdin, crlfDelay: Infinity })) {
    const request = JSON.parse(line);
    fs.appendFileSync(log, JSON.stringify(request) + '\n');
    if (request.method === 'initialize') {
        send({ jsonrpc: '2.0', id: request.id, result: { protocolVersion: request.params.protocolVersion, capabilities: { tools: { listChanged: true } }, serverInfo: { name: 'autocraft-p05-fixture', version: '1' } } });
    } else if (request.method === 'tools/list') {
        const tools = [tool('enable_toolset', { toolset: { type: 'string' } })];
        if (enabled) tools.push(tool('native_functions_list'));
        send({ jsonrpc: '2.0', id: request.id, result: { tools } });
        fs.appendFileSync(log, JSON.stringify({ emitted: 'tools/list result', tool_count: tools.length, enabled }) + '\n');
    } else if (request.method === 'tools/call') {
        if (request.params.name === 'enable_toolset') {
            enabled = true;
            send({ jsonrpc: '2.0', id: request.id, result: { content: [{ type: 'text', text: 'P05_NATIVE_ENABLED' }], isError: false } });
            send({ jsonrpc: '2.0', method: 'notifications/tools/list_changed' });
            fs.appendFileSync(log, JSON.stringify({ emitted: 'notifications/tools/list_changed' }) + '\n');
        } else if (request.params.name === 'native_functions_list') {
            enabled = true;
            send({ jsonrpc: '2.0', id: request.id, result: { content: [{ type: 'text', text: 'P05_NATIVE_RESULT_FROM_MCP' }], isError: false } });
        } else send({ jsonrpc: '2.0', id: request.id, error: { code: -32601, message: 'Unknown fixture tool' } });
    } else if (request.id !== undefined) {
        send({ jsonrpc: '2.0', id: request.id, error: { code: -32601, message: 'Unsupported fixture method' } });
    }
}
