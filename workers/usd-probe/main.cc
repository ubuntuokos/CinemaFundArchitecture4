// SPDX-License-Identifier: GPL-3.0-or-later
// cfa4-usd-probe: one command per process, JSON on stdout, nothing else (the shape of workers/vdb).
//   inspect <file>                 -> {"upAxis":...,"primCount":...}  opens the stage without loading payloads
//   selftest-write <out>           -> {"ok":true}  small scene; the format follows the extension (.usda .usdc .usdz)
//   selftest-write-deep <out> <n>  -> {"ok":true}  a chain of n nested prims (depth tests)
//   selftest-write-cycle <out>     -> {"ok":true}  a .usda whose sublayer is itself (cycle tests)
// Every failure is {"error":"..."} on stdout and exit code 2; the host treats a crash as an error too.
#include <pxr/pxr.h>
#include <pxr/base/tf/diagnostic.h>
#include <pxr/base/tf/errorMark.h>
#include <pxr/base/tf/pathUtils.h>
#include <pxr/usd/sdf/layer.h>
#include <pxr/usd/usd/prim.h>
#include <pxr/usd/usd/primRange.h>
#include <pxr/usd/usd/stage.h>
#include <pxr/usd/usdGeom/metrics.h>
#include <pxr/usd/usdGeom/xform.h>
#include <pxr/usd/usdUtils/dependencies.h>
#include <sys/resource.h>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <exception>
#include <iostream>
#include <sstream>
#include <string>

PXR_NAMESPACE_USING_DIRECTIVE

// Limits (AGENTS.md §0.5): the traversal stops, and says so, past these.
static const size_t kMaxPrims = 1000000;
static const size_t kMaxDepth = 512;
static const size_t kMaxNames = 64;

static std::string esc(const std::string& s) {
    std::string o;
    size_t n = 0;
    for (unsigned char c : s) {
        if (++n > 400) { o += "..."; break; }  // a file can embed its own bytes in a message
        if (c == '"' || c == '\\') { o += '\\'; o += (char)c; }
        else if (c < 0x20 || c >= 0x7f) { char b[8]; std::snprintf(b, sizeof b, "\\u%04x", c); o += b; }
        else o += (char)c;
    }
    return o;
}

static int fail(const std::string& m) {
    std::cout << "{\"error\":\"" << esc(m) << "\"}\n";
    return 2;
}

static long peakRssKb() {
    FILE* f = std::fopen("/proc/self/status", "r");
    if (!f) return 0;
    char line[256];
    long kb = 0;
    while (std::fgets(line, sizeof line, f))
        if (std::sscanf(line, "VmHWM: %ld kB", &kb) == 1) break;
    std::fclose(f);
    return kb;
}

// A hostile file can ask for absurd allocations: cap the address space (CFA4_USD_MEM_LIMIT_MB, default 4096).
static void limitMemory() {
    long mb = 4096;
    if (const char* e = std::getenv("CFA4_USD_MEM_LIMIT_MB")) mb = std::atol(e);
    if (mb <= 0) return;
    struct rlimit rl;
    rl.rlim_cur = rl.rlim_max = (rlim_t)mb * 1024 * 1024;
    setrlimit(RLIMIT_AS, &rl);
}

static std::string inspect(const std::string& path) {
    TfErrorMark mark;
    UsdStageRefPtr stage = UsdStage::Open(path, UsdStage::LoadNone);
    if (!stage) throw std::runtime_error("cannot open the stage");
    if (!mark.IsClean()) {
        std::string msg;
        for (auto it = mark.GetBegin(); it != mark.GetEnd(); ++it) { msg = it->GetCommentary(); break; }
        throw std::runtime_error(msg.empty() ? "USD reported an error" : msg);
    }
    size_t prims = 0, maxDepth = 0;
    bool truncated = false;
    std::ostringstream roots;
    size_t rootCount = 0;
    for (const UsdPrim& root : stage->GetPseudoRoot().GetChildren()) {
        if (rootCount++ < kMaxNames) roots << (rootCount > 1 ? "," : "") << "\"" << esc(root.GetName().GetString()) << "\"";
    }
    UsdPrimRange range = UsdPrimRange::Stage(stage);
    for (auto it = range.begin(); it != range.end(); ++it) {
        const size_t depth = it->GetPath().GetPathElementCount();
        if (depth > maxDepth) maxDepth = depth;
        if (++prims >= kMaxPrims || depth >= kMaxDepth) { truncated = true; break; }
    }
    std::ostringstream o;
    o << "{\"upAxis\":\"" << esc(UsdGeomGetStageUpAxis(stage).GetString()) << "\""
      << ",\"metersPerUnit\":" << UsdGeomGetStageMetersPerUnit(stage)
      << ",\"startTimeCode\":" << stage->GetStartTimeCode() << ",\"endTimeCode\":" << stage->GetEndTimeCode()
      << ",\"timeCodesPerSecond\":" << stage->GetTimeCodesPerSecond()
      << ",\"primCount\":" << prims << ",\"maxDepth\":" << maxDepth << ",\"truncated\":" << (truncated ? "true" : "false")
      << ",\"rootCount\":" << rootCount << ",\"rootPrims\":[" << roots.str() << "]"
      << ",\"layerCount\":" << stage->GetUsedLayers().size() << ",\"peakRssKb\":" << peakRssKb() << "}";
    return o.str();
}

static bool endsWith(const std::string& s, const char* suf) {
    const size_t n = std::strlen(suf);
    return s.size() >= n && s.compare(s.size() - n, n, suf) == 0;
}

// Writes the stage; a .usdz is a package of a temporary .usdc next to it.
static void exportStage(const UsdStageRefPtr& stage, const std::string& out) {
    if (endsWith(out, ".usdz")) {
        const std::string tmp = out + ".tmp.usdc";
        stage->GetRootLayer()->Export(tmp);
        const bool ok = UsdUtilsCreateNewUsdzPackage(SdfAssetPath(tmp), out);
        std::remove(tmp.c_str());
        if (!ok) throw std::runtime_error("cannot write the usdz package");
    } else if (!stage->GetRootLayer()->Export(out)) {
        throw std::runtime_error("cannot write the file");
    }
}

int main(int argc, char** argv) {
    limitMemory();
    try {
        if (argc == 3 && !std::strcmp(argv[1], "inspect")) {
            std::cout << inspect(argv[2]) << "\n";
            return 0;
        }
        if (argc == 3 && !std::strcmp(argv[1], "selftest-write")) {
            UsdStageRefPtr stage = UsdStage::CreateInMemory();
            UsdGeomSetStageUpAxis(stage, UsdGeomTokens->y);
            UsdGeomSetStageMetersPerUnit(stage, 0.01);
            UsdGeomXform::Define(stage, SdfPath("/World"));
            UsdGeomXform::Define(stage, SdfPath("/World/Cam"));
            UsdGeomXform::Define(stage, SdfPath("/World/Lights"));
            exportStage(stage, argv[2]);
            std::cout << "{\"ok\":true}\n";
            return 0;
        }
        if (argc == 4 && !std::strcmp(argv[1], "selftest-write-deep")) {
            const long n = std::atol(argv[3]);
            if (n < 1 || n > 100000) return fail("depth must be 1..100000");
            UsdStageRefPtr stage = UsdStage::CreateInMemory();
            std::string p;
            for (long i = 0; i < n; ++i) { p += "/p"; UsdGeomXform::Define(stage, SdfPath(p)); }
            exportStage(stage, argv[2]);
            std::cout << "{\"ok\":true}\n";
            return 0;
        }
        if (argc == 3 && !std::strcmp(argv[1], "selftest-write-cycle")) {
            const std::string out = argv[2];
            std::FILE* f = std::fopen(out.c_str(), "w");
            if (!f) return fail("cannot write the file");
            std::fprintf(f, "#usda 1.0\n(\n    subLayers = [@%s@]\n)\ndef Xform \"A\"\n{\n}\n", TfGetBaseName(out).c_str());
            std::fclose(f);
            std::cout << "{\"ok\":true}\n";
            return 0;
        }
        return fail("usage: inspect <file> | selftest-write <out> | selftest-write-deep <out> <n> | selftest-write-cycle <out>");
    } catch (const std::bad_alloc&) {
        return fail("out of memory");
    } catch (const std::exception& e) {
        return fail(e.what());
    }
}
