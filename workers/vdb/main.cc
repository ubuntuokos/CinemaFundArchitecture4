// cfa4-vdb-worker: one command per process, JSON on stdout, nothing else.
//   inspect <file>          -> {"grids":[...]}                       (N1-1)
//   copy <in> <out>         -> {"ok":true}  read, write (round trip) (N1-4)
//   selftest-write-big <out> <n>  n^3 dense voxels (size tests)
//   selftest-write <out>    -> {"ok":true}  writes a small synthetic VDB for tests (no binary fixtures in the repo)
// Every failure is {"error":"..."} on stdout and exit code 2; the host treats a crash as an error too.
#include <openvdb/openvdb.h>
#include <openvdb/points/PointDataGrid.h>
#include <openvdb/util/logging.h>
#include <sys/resource.h>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <exception>
#include <iostream>
#include <sstream>
#include <string>

static std::string esc(const std::string& s) {
    std::string o;
    size_t n = 0;
    for (unsigned char c : s) {
        if (++n > 400) { o += "..."; break; }  // OpenVDB can embed file bytes in a message
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

// Peak resident set of this process in KiB (VmHWM), 0 if unknown: the size tests (N1-2) read it back.
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

static std::string inspect(const std::string& path) {
    openvdb::io::File file(path);
    file.setCopyMaxBytes(0);  // never copy the file for delayed loading: the confined worker has no temp directory
    file.open();
    openvdb::GridPtrVecPtr grids = file.getGrids();
    openvdb::MetaMap::Ptr fmeta = file.getMetadata();
    std::ostringstream o;
    o << "{\"grids\":[";
    bool first = true;
    for (const auto& g : *grids) {
        if (!first) o << ",";
        first = false;
        const openvdb::CoordBBox bb = g->evalActiveVoxelBoundingBox();
        const openvdb::Vec3d vs = g->voxelSize();
        o << "{\"name\":\"" << esc(g->getName()) << "\",\"type\":\"" << esc(g->valueType())
          << "\",\"gridClass\":\"" << esc(openvdb::GridBase::gridClassToString(g->getGridClass()))
          << "\",\"voxelSize\":[" << vs[0] << "," << vs[1] << "," << vs[2] << "]"
          << ",\"activeVoxels\":" << g->activeVoxelCount()
          << ",\"bbox\":{\"min\":[" << bb.min()[0] << "," << bb.min()[1] << "," << bb.min()[2]
          << "],\"max\":[" << bb.max()[0] << "," << bb.max()[1] << "," << bb.max()[2] << "]}"
          << ",\"isPoints\":" << (g->isType<openvdb::points::PointDataGrid>() ? "true" : "false")
          << ",\"meta\":{";
        bool fm = true;
        for (auto it = g->beginMeta(); it != g->endMeta(); ++it) {
            if (!fm) o << ",";
            fm = false;
            o << "\"" << esc(it->first) << "\":\"" << esc(it->second->str()) << "\"";
        }
        o << "}}";
    }
    o << "],\"fileMetaCount\":" << (fmeta ? fmeta->metaCount() : 0) << ",\"peakRssKb\":" << peakRssKb() << "}";
    file.close();
    return o.str();
}

// A hostile file can ask for absurd allocations: cap the address space (CFA4_VDB_MEM_LIMIT_MB, default 4096) so
// the worker fails with std::bad_alloc, reported as an error, instead of starving the machine (N1-3).
static void limitMemory() {
    long mb = 4096;
    if (const char* e = std::getenv("CFA4_VDB_MEM_LIMIT_MB")) mb = std::atol(e);
    if (mb <= 0) return;
    struct rlimit rl;
    rl.rlim_cur = rl.rlim_max = (rlim_t)mb * 1024 * 1024;
    setrlimit(RLIMIT_AS, &rl);
}

int main(int argc, char** argv) {
    limitMemory();
    try {
        openvdb::initialize();
        // OpenVDB's logger writes warnings to stdout, which is the JSON channel: silence it.
#ifdef OPENVDB_USE_LOG4CPLUS
        openvdb::logging::setLevel(openvdb::logging::Level::Fatal);
#endif
        if (argc == 3 && !std::strcmp(argv[1], "inspect")) {
            std::cout << inspect(argv[2]) << "\n";
            return 0;
        }
        if (argc == 4 && !std::strcmp(argv[1], "copy")) {
            openvdb::io::File in(argv[2]);
            in.setCopyMaxBytes(0);
            in.open();
            openvdb::GridPtrVecPtr grids = in.getGrids();
            openvdb::MetaMap::Ptr meta = in.getMetadata();
            in.close();
            openvdb::io::File out(argv[3]);
            out.write(*grids, *meta);
            out.close();
            std::cout << "{\"ok\":true}\n";
            return 0;
        }
        if (argc == 3 && !std::strcmp(argv[1], "selftest-write")) {
            auto f = openvdb::FloatGrid::create(0.0f);
            f->setName("density");
            f->setTransform(openvdb::math::Transform::createLinearTransform(0.5));
            auto acc = f->getAccessor();
            for (int i = 0; i < 20; ++i)
                for (int j = 0; j < 20; ++j)
                    for (int k = 0; k < 20; ++k) acc.setValue(openvdb::Coord(i, j, k), float(i + j + k));
            auto v = openvdb::Vec3SGrid::create(openvdb::Vec3f(0));
            v->setName("vel");
            v->tree().setValue(openvdb::Coord(1, 2, 3), openvdb::Vec3f(1, 2, 3));
            openvdb::GridPtrVec gs{f, v};
            openvdb::io::File file(argv[2]);
            file.write(gs);
            file.close();
            std::cout << "{\"ok\":true}\n";
            return 0;
        }
        if (argc == 4 && !std::strcmp(argv[1], "selftest-write-big")) {
            // n^3 dense float voxels (n = 650 is about 1 GiB of active data), written leaf by leaf for the size tests
            const int n = std::atoi(argv[3]);
            if (n < 1 || n > 2000) return fail("n out of range");
            auto f = openvdb::FloatGrid::create(0.0f);
            f->setName("big");
            f->denseFill(openvdb::CoordBBox(openvdb::Coord(0), openvdb::Coord(n - 1)), 1.0f, true);
            // denseFill makes tiles when it can; touch every leaf so the data is really dense
            f->tree().voxelizeActiveTiles();
            openvdb::GridPtrVec gs{f};
            openvdb::io::File file(argv[2]);
            file.write(gs);
            file.close();
            std::cout << "{\"ok\":true}\n";
            return 0;
        }
        return fail("usage: inspect <file> | copy <in> <out> | selftest-write <out>");
    } catch (const std::exception& e) {
        return fail(e.what());
    } catch (...) {
        return fail("unknown error");
    }
}
