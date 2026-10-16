/** Node-only installed gateway discovery, selected by the package import map. */
import {closeSync, constants, fstatSync, openSync, readSync} from 'node:fs';
import {homedir} from 'node:os';
import {join} from 'node:path';
import process from 'node:process';

const ERROR = 'Cannot read local Capsem gateway metadata. Start Capsem and try again; check CAPSEM_RUN_DIR or CAPSEM_HOME if customized.';

function read(path: string, privateFile = false): string {
  // Node opens descriptors close-on-exec; reject links and non-regular files.
  const descriptor = openSync(path, constants.O_RDONLY | constants.O_NOFOLLOW | constants.O_NONBLOCK);
  try {
    const info = fstatSync(descriptor);
    if (!info.isFile() || info.uid !== process.getuid?.() || (privateFile && (info.mode & 0o077))) {
      throw new Error('Metadata must be an owned regular file, with private credentials');
    }
    const buffer = Buffer.alloc(4097);
    let size = 0;
    while (size < buffer.length) {
      const count = readSync(descriptor, buffer, size, buffer.length - size, null);
      if (!count) break;
      size += count;
    }
    if (size > 4096 || buffer.subarray(0, size).some(byte => byte > 127)) throw new Error('Invalid metadata');
    return buffer.subarray(0, size).toString('ascii').trim();
  } finally {closeSync(descriptor);}
}

export function localConnection(): [string, string] {
  try {
    const root = process.env.CAPSEM_RUN_DIR || join(process.env.CAPSEM_HOME || join(homedir(), '.capsem'), 'run');
    const token = read(join(root, 'gateway.token'), true);
    const port = read(join(root, 'gateway.port'));
    if (!/^\d+$/.test(port) || Number(port) < 1 || Number(port) > 65535 || !/^[\x21-\x7e]+$/.test(token)) {
      throw new Error('Invalid gateway metadata');
    }
    return [`http://127.0.0.1:${Number(port)}`, token];
  } catch {throw new Error(ERROR);}
}
