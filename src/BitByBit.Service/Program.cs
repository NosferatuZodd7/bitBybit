// bitBybit broker service entry point. The full host (IPC server, operation handlers, watchdogs, recovery) is wired in
// ServiceHost.cs; this file only selects the mode.
using BitByBit.Service;

return await ServiceEntryPoint.RunAsync(args).ConfigureAwait(false);
