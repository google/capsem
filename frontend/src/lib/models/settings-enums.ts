// Settings grammar enums -- must match Rust serde serialization exactly.
// No string comparison in UI code: always use these enum values.

export enum SettingType {
  Text = 'text',
  Number = 'number',
  Bool = 'bool',
  Password = 'password',
  ApiKey = 'apikey',
  Url = 'url',
  Email = 'email',
  File = 'file',
  StringList = 'string_list',
  IntList = 'int_list',
  FloatList = 'float_list',
}

export enum Widget {
  Toggle = 'toggle',
  TextInput = 'text_input',
  NumberInput = 'number_input',
  PasswordInput = 'password_input',
  Select = 'select',
  FileEditor = 'file_editor',
  DomainChips = 'domain_chips',
  StringChips = 'string_chips',
  Slider = 'slider',
}

export enum SideEffect {
  ToggleTheme = 'toggle_theme',
}

export enum ActionKind {
  CheckUpdate = 'check_update',
  PresetSelect = 'preset_select',
  RerunWizard = 'rerun_wizard',
}

export enum McpTransport {
  Stdio = 'stdio',
  Sse = 'sse',
}

export enum PolicySource {
  Default = 'default',
  User = 'user',
  Corp = 'corp',
}

/** Map SettingType to its default Widget (no string comparison). */
export function defaultWidget(type: SettingType): Widget {
  switch (type) {
    case SettingType.Bool:
      return Widget.Toggle;
    case SettingType.Number:
      return Widget.NumberInput;
    case SettingType.Password:
    case SettingType.ApiKey:
      return Widget.PasswordInput;
    case SettingType.File:
      return Widget.FileEditor;
    case SettingType.StringList:
      return Widget.StringChips;
    case SettingType.IntList:
    case SettingType.FloatList:
      return Widget.NumberInput;
    case SettingType.Text:
    case SettingType.Url:
    case SettingType.Email:
      return Widget.TextInput;
  }
}
