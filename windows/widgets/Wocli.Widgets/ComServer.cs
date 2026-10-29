using System.Runtime.InteropServices;
using Microsoft.Windows.Widgets.Providers;
using WinRT;

namespace Wocli.Widgets;

[ComImport]
[InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
[Guid("00000001-0000-0000-C000-000000000046")]
public interface IClassFactory
{
    [PreserveSig]
    int CreateInstance(nint outer, ref Guid iid, out nint result);
    [PreserveSig]
    int LockServer([MarshalAs(UnmanagedType.Bool)] bool locked);
}

[ComVisible(true)]
[ClassInterface(ClassInterfaceType.None)]
[ComDefaultInterface(typeof(IClassFactory))]
public sealed class WidgetClassFactory(WidgetProvider provider, Action<string>? diagnostics = null) : IClassFactory
{
    public int CreateInstance(nint outer, ref Guid iid, out nint result)
    {
        result = 0;
        if (outer != 0)
            return unchecked((int)0x80040110); // CLASS_E_NOAGGREGATION
        nint inspectable = 0;
        try
        {
            diagnostics?.Invoke($"factory.CreateInstance requested IID={iid:D}");
            inspectable = MarshalInspectable<IWidgetProvider>.FromManaged(provider);
            diagnostics?.Invoke("factory.FromManaged succeeded");
            // Return the requested interface, rather than treating a CLSID as an IID.
            int status = Marshal.QueryInterface(inspectable, ref iid, out result);
            diagnostics?.Invoke($"factory.QueryInterface HRESULT=0x{status:X8}, nonNull={result != 0}");
            return status;
        }
        catch (Exception error)
        {
            diagnostics?.Invoke($"factory exception: {error.GetType().FullName}, HRESULT=0x{error.HResult:X8}: {error}");
            return error.HResult;
        }
        finally
        {
            if (inspectable != 0)
                Marshal.Release(inspectable);
        }
    }

    // This local server remains alive until its stop event is signalled. It is revoked
    // explicitly at shutdown, after which no new objects can be activated.
    public int LockServer(bool locked) => 0;
}

internal sealed class ComServer : IDisposable
{
    private readonly WidgetClassFactory _factory;
    private uint _cookie;

    public ComServer(WidgetProvider provider, Guid? classId = null, Action<string>? diagnostics = null)
    {
        _factory = new WidgetClassFactory(provider, diagnostics);
        Marshal.ThrowExceptionForHR(CoRegisterClassObject(
            classId ?? new Guid(ProviderIdentity.ClassId), _factory, 4, 1, out _cookie));
    }

    public void Dispose()
    {
        if (_cookie == 0) return;
        int result = CoRevokeClassObject(_cookie);
        _cookie = 0;
        GC.KeepAlive(_factory);
        if (result < 0)
            ProviderLog.Error("revoke_class", Marshal.GetExceptionForHR(result)!);
    }

    [DllImport("ole32.dll", ExactSpelling = true)]
    private static extern int CoRegisterClassObject(
        [MarshalAs(UnmanagedType.LPStruct)] Guid classId,
        [MarshalAs(UnmanagedType.IUnknown)] object factory,
        uint context, uint flags, out uint cookie);

    [DllImport("ole32.dll", ExactSpelling = true)]
    private static extern int CoRevokeClassObject(uint cookie);
}

internal sealed class ComApartment : IDisposable
{
    public ComApartment() => Marshal.ThrowExceptionForHR(CoInitializeEx(0, 0));
    public void Dispose() => CoUninitialize();

    [DllImport("ole32.dll", ExactSpelling = true)]
    private static extern int CoInitializeEx(nint reserved, uint flags);
    [DllImport("ole32.dll", ExactSpelling = true)]
    private static extern void CoUninitialize();
}
