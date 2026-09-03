// Windows package smoke test only. This is not the application's DLL loader.
using System;
using System.ComponentModel;
using System.Diagnostics;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;

public static class NativeRuntimeProbe
{
    private const uint System32 = 0x00000800;
    private const uint DllLoadDir = 0x00000100;

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool SetDefaultDllDirectories(uint flags);
    [DllImport("kernel32.dll")]
    private static extern uint SetErrorMode(uint mode);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern IntPtr LoadLibraryExW(string path, IntPtr file, uint flags);
    [DllImport("kernel32.dll", CharSet = CharSet.Ansi, ExactSpelling = true, SetLastError = true)]
    private static extern IntPtr GetProcAddress(IntPtr module, string name);
    [DllImport("kernel32.dll")]
    private static extern bool FreeLibrary(IntPtr module);

    // Signatures match the pinned llama.h, ggml-backend.h and ggml-cpu.h.
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate void VoidCall();
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate IntPtr CpuInit();
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate IntPtr AllocBuffer(IntPtr backend, UIntPtr size);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate UIntPtr BufferSize(IntPtr buffer);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate void Release(IntPtr value);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate IntPtr LoadBackend([MarshalAs(UnmanagedType.LPUTF8Str)] string path);

    public static void BreakVulkanImport(string path)
    {
        // Fault injection on a temporary unsigned DLL copy only. Same byte length
        // preserves PE offsets and forces a missing transitive dependency even on
        // hosts that have a working system Vulkan loader.
        byte[] bytes = File.ReadAllBytes(path);
        string contents = Encoding.Latin1.GetString(bytes);
        const string original = "vulkan-1.dll\0", missing = "omai-mis.dll\0";
        int offset = contents.IndexOf(original, StringComparison.OrdinalIgnoreCase);
        if (offset < 0 || contents.IndexOf(original, offset + 1, StringComparison.OrdinalIgnoreCase) >= 0)
            throw new InvalidOperationException("Expected exactly one Vulkan loader import name");
        Encoding.Latin1.GetBytes(missing).CopyTo(bytes, offset);
        File.WriteAllBytes(path, bytes);
    }

    private static T Export<T>(IntPtr module, string name) where T : Delegate
    {
        IntPtr address = GetProcAddress(module, name);
        if (address == IntPtr.Zero)
            throw new InvalidOperationException("Missing native export: " + name);
        return Marshal.GetDelegateForFunctionPointer<T>(address);
    }

    private static IntPtr Load(string directory, string name)
    {
        // Absolute bundle path plus System32: never PATH, CWD, SDK or build output.
        IntPtr module = LoadLibraryExW(Path.Combine(directory, name), IntPtr.Zero,
                                      DllLoadDir | System32);
        if (module == IntPtr.Zero)
            throw new Win32Exception(Marshal.GetLastWin32Error(), "Cannot load " + name);
        return module;
    }

    public static int Run(string directory, bool dynamicBackends, bool cpuOnly)
    {
        try { return RunCore(directory, dynamicBackends, cpuOnly); }
        catch (Win32Exception error) when (error.NativeErrorCode == 126)
        {
            Console.Error.WriteLine("runtime.load: missing DLL (Win32 126)");
            return 20;
        }
    }

    private static int RunCore(string directory, bool dynamicBackends, bool cpuOnly)
    {
        // Missing DLLs must produce an exit code, not a blocking Windows dialog.
        SetErrorMode(0x0001 | 0x0002 | 0x8000);
        if (!SetDefaultDllDirectories(System32))
            throw new Win32Exception(Marshal.GetLastWin32Error());

        IntPtr llama = Load(directory, "llama.dll");
        IntPtr cpu = IntPtr.Zero, ggmlBase = IntPtr.Zero, ggml = IntPtr.Zero, vulkan = IntPtr.Zero;
        VoidCall shutdown = null;
        bool initialized = false;
        try
        {
            shutdown = Export<VoidCall>(llama, "llama_backend_free");
            cpu = Load(directory, "ggml-cpu.dll");
            if (dynamicBackends)
            {
                ggml = Load(directory, "ggml.dll");
                var register = Export<LoadBackend>(ggml, "ggml_backend_load");
                if (register(Path.Combine(directory, "ggml-cpu.dll")) == IntPtr.Zero)
                    throw new InvalidOperationException("CPU backend registration failed");
                if (!cpuOnly)
                {
                    try { vulkan = Load(directory, "ggml-vulkan.dll"); }
                    catch (Win32Exception error) when (error.NativeErrorCode == 126)
                    {
                        Console.WriteLine("runtime.vulkan: unavailable (Win32 126); CPU remains usable");
                    }
                    if (vulkan != IntPtr.Zero && register(Path.Combine(directory, "ggml-vulkan.dll")) == IntPtr.Zero)
                        throw new InvalidOperationException("Vulkan backend registration failed");
                }
            }
            Export<VoidCall>(llama, "llama_backend_init")();
            initialized = true;
            Console.WriteLine("runtime.init: passed");

            ggmlBase = Load(directory, "ggml-base.dll");
            var freeBackend = Export<Release>(ggmlBase, "ggml_backend_free");
            var freeBuffer = Export<Release>(ggmlBase, "ggml_backend_buffer_free");
            IntPtr backend = Export<CpuInit>(cpu, "ggml_backend_cpu_init")();
            if (backend == IntPtr.Zero) throw new InvalidOperationException("CPU init failed");
            try
            {
                IntPtr buffer = Export<AllocBuffer>(ggmlBase, "ggml_backend_alloc_buffer")(
                    backend, new UIntPtr(4096));
                if (buffer == IntPtr.Zero) throw new InvalidOperationException("CPU allocation failed");
                try
                {
                    if (Export<BufferSize>(ggmlBase, "ggml_backend_buffer_get_size")(buffer).ToUInt64() < 4096)
                        throw new InvalidOperationException("CPU buffer is too small");
                }
                finally { freeBuffer(buffer); }
            }
            finally { freeBackend(backend); }
            Console.WriteLine("runtime.cpu-buffer: passed (4096 bytes)");

            // Record the actual Vulkan loader / ggml locations for diagnosis.
            foreach (ProcessModule module in Process.GetCurrentProcess().Modules)
            {
                string name = module.ModuleName;
                if (dynamicBackends && cpuOnly &&
                    (name.Equals("ggml-vulkan.dll", StringComparison.OrdinalIgnoreCase) ||
                     name.Equals("vulkan-1.dll", StringComparison.OrdinalIgnoreCase)))
                    throw new InvalidOperationException("CPU-only initialization loaded Vulkan");
                if (name.StartsWith("ggml", StringComparison.OrdinalIgnoreCase) ||
                    name.Equals("llama.dll", StringComparison.OrdinalIgnoreCase) ||
                    name.Equals("vulkan-1.dll", StringComparison.OrdinalIgnoreCase))
                    Console.WriteLine("runtime.module: " + module.FileName);
            }
            return 0;
        }
        finally
        {
            if (initialized) shutdown();
            if (ggmlBase != IntPtr.Zero) FreeLibrary(ggmlBase);
            if (cpu != IntPtr.Zero) FreeLibrary(cpu);
            if (vulkan != IntPtr.Zero) FreeLibrary(vulkan);
            if (ggml != IntPtr.Zero) FreeLibrary(ggml);
            FreeLibrary(llama);
        }
    }
}
