# Match Rust's default dynamically linked MSVC runtime. The library itself is
# still statically linked into mtp-cull, so no turbojpeg DLL is shipped.
set(WITH_CRT_DLL ON CACHE BOOL "Use the MSVC DLL runtime" FORCE)
set(CMAKE_MSVC_RUNTIME_LIBRARY MultiThreadedDLL CACHE STRING "Match Rust CRT in every profile" FORCE)
