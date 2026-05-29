@external("capsem", "emit_output")
declare function abiEmitOutput(ptr: usize, len: i32): i32;

@external("capsem", "fs_read")
declare function abiFsRead(ptr: usize, len: i32): i32;

@external("capsem", "read_host_response")
declare function abiReadHostResponse(ptr: usize, len: i32): i32;

@external("capsem", "emit_ui_block")
declare function abiUiEmit(ptr: usize, len: i32): i32;

@external("capsem", "fetch")
declare function abiFetch(ptr: usize, len: i32): i32;

type FileCreateCallback = (file_event: FileCreate, context: Context) => FileCreate;

class PluginRuntime {
  private onFileCreateCallback: FileCreateCallback | null = null;

  constructor(readonly name: string) {}

  on_file_create(callback: FileCreateCallback): PluginRuntime {
    this.onFileCreateCallback = callback;
    return this;
  }

  dispatch_file_create(objectPtr: i32, objectLen: i32, contextPtr: i32, contextLen: i32): i32 {
    if (objectPtr < 0 || objectLen < 0 || contextPtr < 0 || contextLen < 0) {
      return -1;
    }

    const callback = this.onFileCreateCallback;
    if (callback === null) {
      return -2;
    }

    const file_event = FileCreate.from_abi(objectPtr, objectLen);
    const context = Context.from_abi(contextPtr, contextLen);
    return callback(file_event, context).emit(70);
  }
}

export function Plugin(name: string): PluginRuntime {
  return new PluginRuntime(name);
}

class Context {
  readonly fs: Fs;
  readonly ui: Ui;

  private constructor(readonly raw_json: string) {
    this.fs = new Fs();
    this.ui = new Ui();
  }

  static from_abi(ptr: i32, len: i32): Context {
    return new Context(String.UTF8.decodeUnsafe(ptr, len, false));
  }

  fetch(url: string): string {
    const urlBytes = String.UTF8.encode(url, false);
    const responseLen = abiFetch(changetype<usize>(urlBytes), urlBytes.byteLength);
    if (responseLen <= 0) {
      return "";
    }

    const response = new ArrayBuffer(responseLen);
    const copied = abiReadHostResponse(changetype<usize>(response), responseLen);
    if (copied <= 0) {
      return "";
    }

    return String.UTF8.decodeUnsafe(changetype<usize>(response), copied, false);
  }
}

class Fs {
  read(path: string): string {
    const pathBytes = String.UTF8.encode(path, false);
    const responseLen = abiFsRead(changetype<usize>(pathBytes), pathBytes.byteLength);
    if (responseLen <= 0) {
      return "";
    }

    const response = new ArrayBuffer(responseLen);
    const copied = abiReadHostResponse(changetype<usize>(response), responseLen);
    if (copied <= 0) {
      return "";
    }

    return String.UTF8.decodeUnsafe(changetype<usize>(response), copied, false);
  }
}

class Ui {
  emit(mutation: UiMutation): void {
    const bytes = String.UTF8.encode(mutation.to_json(), false);
    abiUiEmit(changetype<usize>(bytes), bytes.byteLength);
  }
}

class UiMutation {
  constructor(
    readonly channel: string,
    readonly operation: string,
    readonly block: UiBlock,
  ) {}

  to_json(): string {
    return (
      "{" +
      '"channel":' + json_string(this.channel) + "," +
      '"operation":' + json_string(this.operation) + "," +
      '"block":' + this.block.to_json() +
      "}"
    );
  }
}

class UiBlock {
  constructor(
    readonly kind: string,
    readonly id: string,
    readonly title: string,
    readonly subtitle: string,
    readonly body: string,
  ) {}

  to_json(): string {
    return (
      "{" +
      '"kind":' + json_string(this.kind) + "," +
      '"id":' + json_string(this.id) + "," +
      '"title":' + json_string(this.title) + "," +
      '"subtitle":' + json_string(this.subtitle) + "," +
      '"body":' + json_string(this.body) +
      "}"
    );
  }
}

class FileCreate {
  private constructor(
    readonly path: string,
    readonly action: string,
  ) {}

  static from_abi(ptr: i32, len: i32): FileCreate {
    const raw_json = String.UTF8.decodeUnsafe(ptr, len, false);
    return new FileCreate(
      extract_json_string(raw_json, "path"),
      extract_json_string(raw_json, "action"),
    );
  }

  emit(status: i32): i32 {
    const output =
      "{" +
      '"kind":"FileCreate",' +
      '"path":' + json_string(this.path) + "," +
      '"action":' + json_string(this.action) + "," +
      '"decision":{"verdict":"allow","reasons":[]},' +
      '"patches":[],' +
      '"findings":[]' +
      "}";
    emit_json(output);
    return status;
  }
}

const plugin = Plugin("capsem.git-context").on_file_create((file_event, context) => {
  if (file_event.path != ".git" && !file_event.path.endsWith("/.git")) {
    return file_event;
  }

  const head = context.fs.read(file_event.path + "/HEAD");
  if (head.length == 0) {
    return file_event;
  }

  const branch = parse_branch(head);
  const worktree = parent_dir(file_event.path);
  const project = basename(worktree);
  const config = context.fs.read(file_event.path + "/config");
  const github_api_url = github_repo_api_url(config);
  const github_stats = github_api_url.length > 0
    ? parse_github_stats(context.fetch(github_api_url))
    : "github stats unavailable";

  context.ui.emit(new UiMutation(
    "workspace.context",
    "upsert_block",
    new UiBlock(
      "git-context-card",
      "git-context:" + worktree,
      project,
      branch,
      worktree + " | " + github_stats,
    ),
  ));

  return file_event;
});

export function on_file_create(objectPtr: i32, objectLen: i32, contextPtr: i32, contextLen: i32): i32 {
  return plugin.dispatch_file_create(objectPtr, objectLen, contextPtr, contextLen);
}

function emit_json(json: string): void {
  const bytes = String.UTF8.encode(json, false);
  abiEmitOutput(changetype<usize>(bytes), bytes.byteLength);
}

function parse_branch(head: string): string {
  const prefix = "ref: refs/heads/";
  if (head.startsWith(prefix)) {
    return trim(head.substring(prefix.length));
  }
  return "detached";
}

function github_repo_api_url(config: string): string {
  const marker = "github.com";
  const marker_index = config.indexOf(marker);
  if (marker_index < 0) {
    return "";
  }

  let start = marker_index + marker.length;
  if (start < config.length && (config.charAt(start) == "/" || config.charAt(start) == ":")) {
    start++;
  }

  let end = start;
  while (end < config.length) {
    const ch = config.charAt(end);
    if (ch == "\n" || ch == "\r" || ch == " " || ch == "\t") {
      break;
    }
    end++;
  }

  let repo = config.substring(start, end);
  if (repo.endsWith(".git")) {
    repo = repo.substring(0, repo.length - 4);
  }
  if (repo.indexOf("/") < 0) {
    return "";
  }

  return "https://api.github.com/repos/" + repo;
}

function parse_github_stats(json: string): string {
  if (json.length == 0) {
    return "github stats unavailable";
  }
  return (
    "stars " + extract_json_number(json, "stargazers_count") +
    " / forks " + extract_json_number(json, "forks_count") +
    " / issues " + extract_json_number(json, "open_issues_count")
  );
}

function extract_json_number(json: string, key: string): string {
  const needle = '"' + key + '":';
  let index = json.indexOf(needle);
  if (index < 0) {
    return "0";
  }
  index += needle.length;
  while (index < json.length && (json.charAt(index) == " " || json.charAt(index) == "\n")) {
    index++;
  }

  let out = "";
  while (index < json.length) {
    const ch = json.charAt(index);
    if (ch < "0" || ch > "9") {
      break;
    }
    out += ch;
    index++;
  }
  return out.length > 0 ? out : "0";
}

function parent_dir(path: string): string {
  const slash = path.lastIndexOf("/");
  if (slash < 0) {
    return "";
  }
  return path.substring(0, slash);
}

function basename(path: string): string {
  const slash = path.lastIndexOf("/");
  if (slash < 0) {
    return path;
  }
  return path.substring(slash + 1);
}

function extract_json_string(json: string, key: string): string {
  const needle = '"' + key + '":';
  let index = json.indexOf(needle);
  if (index < 0) {
    return "";
  }
  index += needle.length;
  while (index < json.length && (json.charAt(index) == " " || json.charAt(index) == "\n")) {
    index++;
  }
  if (index >= json.length || json.charAt(index) != '"') {
    return "";
  }
  index++;

  let out = "";
  while (index < json.length) {
    const ch = json.charAt(index);
    if (ch == '"') {
      break;
    }
    out += ch;
    index++;
  }
  return out;
}

function trim(value: string): string {
  let start = 0;
  let end = value.length;
  while (start < end && (value.charAt(start) == " " || value.charAt(start) == "\n" || value.charAt(start) == "\r" || value.charAt(start) == "\t")) {
    start++;
  }
  while (end > start && (value.charAt(end - 1) == " " || value.charAt(end - 1) == "\n" || value.charAt(end - 1) == "\r" || value.charAt(end - 1) == "\t")) {
    end--;
  }
  return value.substring(start, end);
}

function json_string(value: string): string {
  let out = '"';
  for (let i = 0; i < value.length; i++) {
    const ch = value.charAt(i);
    if (ch == '"' || ch == "\\") {
      out += "\\" + ch;
    } else if (ch == "\n") {
      out += "\\n";
    } else if (ch == "\r") {
      out += "\\r";
    } else if (ch == "\t") {
      out += "\\t";
    } else {
      out += ch;
    }
  }
  out += '"';
  return out;
}
