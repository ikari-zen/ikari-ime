using System.Runtime.InteropServices;

// P/Invoke の DLL の探索先を、system32 と実行ファイルのフォルダだけにする（カレントディレクトリと PATH を見ない。
// ADR 0067 §4.4）。user32 / dwmapi は system32、核の DLL は実行ファイルのフォルダにある。
// この属性はクラスには付けられない（アセンブリかメソッドだけ）ので、アセンブリに 1 つ付ける
[assembly: DefaultDllImportSearchPaths(DllImportSearchPath.System32 | DllImportSearchPath.ApplicationDirectory)]
