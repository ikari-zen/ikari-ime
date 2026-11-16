using System.Runtime.CompilerServices;
using System.Runtime.InteropServices;
using Ikari.Candidate.V1;
using Microsoft.UI.Dispatching;

namespace IkariCandidate;

/// <summary>
/// 核を起動し、核のスレッドから届く「描く状態」を UI のスレッドに渡す（ADR 0062 §7.3）。
/// 届いたものの最後だけを描く（描く前に次が届いたら、前のものは飛ばす）。判断はしない。
/// </summary>
internal sealed unsafe class CoreHost : IDisposable
{
    private readonly DispatcherQueue _ui;
    private readonly Action<RenderState> _render;
    private GCHandle _self;
    private IntPtr _core;
    /// <summary>まだ描いていない最新の状態（核のスレッドが置き、UI のスレッドが取る）</summary>
    private byte[]? _pending;
    private int _drainQueued;

    private CoreHost(DispatcherQueue ui, Action<RenderState> render)
    {
        _ui = ui;
        _render = render;
    }

    /// <summary>核を起動する。核が受け付けを始められなければ null。</summary>
    public static CoreHost? Start(DispatcherQueue ui, Action<RenderState> render)
    {
        if (CoreNative.GetAbiVersion() != CoreNative.AbiVersion)
        {
            return null;
        }
        var host = new CoreHost(ui, render);
        host._self = GCHandle.Alloc(host);
        host._core = CoreNative.Start(&OnRenderState, GCHandle.ToIntPtr(host._self));
        if (host._core == IntPtr.Zero)
        {
            host._self.Free();
            return null;
        }
        return host;
    }

    /// <summary>核のスレッドから呼ばれる。例外を核に返さない。</summary>
    [UnmanagedCallersOnly(CallConvs = new[] { typeof(CallConvCdecl) })]
    private static void OnRenderState(byte* state, nuint len, IntPtr userData)
    {
        try
        {
            if (len > int.MaxValue || GCHandle.FromIntPtr(userData).Target is not CoreHost host)
            {
                return;
            }
            // 借用はこの呼び出しの間だけ。写してから戻る
            host.Deliver(new ReadOnlySpan<byte>(state, (int)len).ToArray());
        }
        catch (Exception)
        {
            // 描けないだけ。次の状態で描き直す
        }
    }

    private void Deliver(byte[] bytes)
    {
        Volatile.Write(ref _pending, bytes);
        if (Interlocked.Exchange(ref _drainQueued, 1) == 0 && !_ui.TryEnqueue(Drain))
        {
            Volatile.Write(ref _drainQueued, 0);
        }
    }

    private void Drain()
    {
        Volatile.Write(ref _drainQueued, 0);
        byte[]? bytes = Interlocked.Exchange(ref _pending, null);
        if (bytes is null)
        {
            return;
        }
        _render(RenderState.Parser.ParseFrom(bytes));
    }

    public void Dispose()
    {
        if (_core != IntPtr.Zero)
        {
            CoreNative.Stop(_core);
            _core = IntPtr.Zero;
        }
        if (_self.IsAllocated)
        {
            _self.Free();
        }
    }
}
