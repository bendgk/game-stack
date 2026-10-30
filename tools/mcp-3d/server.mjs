#!/usr/bin/env node
// gs3d MCP server (stdio). Registered in the repo's .mcp.json; see README.md.
import { BlenderBridge, serve } from './lib.mjs';

const bridge = new BlenderBridge({
  log: process.env.GS3D_DEBUG ? (s) => process.stderr.write(s) : () => {},
});
serve(process.stdin, process.stdout, bridge);
const stop = () => {
  bridge.stop();
  process.exit(0);
};
process.stdin.on('end', stop);
process.on('SIGINT', stop);
process.on('SIGTERM', stop);
