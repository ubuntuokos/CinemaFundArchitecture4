// cfa4-vdb-worker: one command per process, JSON on stdout, nothing else.
//   inspect <file>          -> {"grids":[...]}                       (N1-1)
//   copy <in> <out>         -> {"ok":true}  read, write (round trip) (N1-4)
//   render <in> <grid|-> <out.png> <w> <h> <preview|standard|high> [az el]  density preview, RGBA PNG (N2)
//   export-tiles <in> <grid> <out.tiles>      one float or vec3s grid as leaf tiles for the CFX bridge (X2)
//   import-tiles <ref.vdb> <ref-grid> <out.vdb> (<name> <tiles>)...   ref.vdb's grids plus new grids made from tiles (X2)
//   selftest-write-big <out> <n>  n^3 dense voxels (size tests)
//   selftest-write <out>    -> {"ok":true}  writes a small synthetic VDB for tests (no binary fixtures in the repo)
// Every failure is {"error":"..."} on stdout and exit code 2; the host treats a crash as an error too.
#include <openvdb/openvdb.h>
#include <openvdb/points/PointDataGrid.h>
#include <openvdb/util/logging.h>
#include <openvdb/tools/Interpolation.h>
#include <tbb/parallel_for.h>
#include <tbb/blocked_range.h>
#include <zlib.h>
#include <cmath>
#include <cstdint>
#include <map>
#include <fstream>
#include <vector>
#include <sys/resource.h>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <exception>
#include <iostream>
#include <sstream>
#include <stdexcept>
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


// ---------- render: a density preview as an RGBA PNG (N2) ----------
// An orthographic ray march of one float grid: each pixel is independent, so the result does not depend on the
// thread count (N2-2). Colour is a fixed grey-white, alpha is 1 - exp(-optical depth), so the picture composes
// over anything as ordinary media. Quality levels fix the step in voxels (preview 2, standard 1, high 0.5).
struct Quality { const char* name; double step; };
static const Quality QUALITIES[] = {{"preview", 2.0}, {"standard", 1.0}, {"high", 0.5}};

static void put32(std::vector<unsigned char>& v, uint32_t x) {
    for (int i = 3; i >= 0; --i) v.push_back((unsigned char)(x >> (8 * i)));
}
static void chunk(std::vector<unsigned char>& out, const char* type, const std::vector<unsigned char>& data) {
    put32(out, (uint32_t)data.size());
    std::vector<unsigned char> td(type, type + 4);
    td.insert(td.end(), data.begin(), data.end());
    out.insert(out.end(), td.begin(), td.end());
    put32(out, (uint32_t)crc32(0, td.data(), (uInt)td.size()));
}
static void writePng(const std::string& path, int w, int h, const std::vector<unsigned char>& rgba) {
    std::vector<unsigned char> raw;
    raw.reserve((size_t)(w * 4 + 1) * h);
    for (int y = 0; y < h; ++y) {
        raw.push_back(0);
        raw.insert(raw.end(), rgba.begin() + (size_t)y * w * 4, rgba.begin() + (size_t)(y + 1) * w * 4);
    }
    uLongf cl = compressBound((uLong)raw.size());
    std::vector<unsigned char> comp(cl);
    if (compress2(comp.data(), &cl, raw.data(), (uLong)raw.size(), 6) != Z_OK) throw std::runtime_error("zlib failed");
    comp.resize(cl);
    std::vector<unsigned char> out{0x89, 'P', 'N', 'G', 0x0d, 0x0a, 0x1a, 0x0a};
    std::vector<unsigned char> ihdr;
    put32(ihdr, (uint32_t)w);
    put32(ihdr, (uint32_t)h);
    ihdr.insert(ihdr.end(), {8, 6, 0, 0, 0});
    chunk(out, "IHDR", ihdr);
    chunk(out, "IDAT", comp);
    chunk(out, "IEND", {});
    FILE* f = std::fopen(path.c_str(), "wb");
    if (!f) throw std::runtime_error("cannot write " + path);
    size_t n = std::fwrite(out.data(), 1, out.size(), f);
    if (std::fclose(f) != 0 || n != out.size()) throw std::runtime_error("short write " + path);
}

static std::string render(const std::string& in, const std::string& gridName, const std::string& outPath, int w, int h,
                          const std::string& quality, double azimuthDeg, double elevationDeg) {
    if (w < 1 || h < 1 || w > 8192 || h > 8192) throw std::runtime_error("size out of range");
    const Quality* q = nullptr;
    for (const auto& c : QUALITIES) if (quality == c.name) q = &c;
    if (!q) throw std::runtime_error("quality must be preview, standard or high");
    openvdb::io::File file(in);
    file.setCopyMaxBytes(0);
    file.open();
    openvdb::FloatGrid::Ptr grid;
    for (auto it = file.beginName(); it != file.endName(); ++it) {
        if (!gridName.empty() && *it != gridName) continue;
        openvdb::GridBase::Ptr g = file.readGrid(*it);
        if (g && g->isType<openvdb::FloatGrid>()) { grid = openvdb::gridPtrCast<openvdb::FloatGrid>(g); break; }
        if (!gridName.empty()) throw std::runtime_error("grid '" + gridName + "' is not a float grid");
    }
    file.close();
    if (!grid) throw std::runtime_error("no float grid to render");
    const openvdb::CoordBBox ib = grid->evalActiveVoxelBoundingBox();
    if (ib.empty()) throw std::runtime_error("the grid has no active voxels");
    const openvdb::BBoxd wb = grid->transform().indexToWorld(openvdb::BBoxd(ib.min().asVec3d() - openvdb::Vec3d(1), ib.max().asVec3d() + openvdb::Vec3d(2)));
    const openvdb::Vec3d centre = wb.getCenter();
    const double radius = 0.5 * wb.extents().length();
    const double vox = grid->voxelSize()[0];
    const double step = q->step * vox;
    const double az = azimuthDeg * M_PI / 180.0, el = elevationDeg * M_PI / 180.0;
    const openvdb::Vec3d dir(-std::cos(el) * std::sin(az), -std::sin(el), -std::cos(el) * std::cos(az));  // looking at the centre
    openvdb::Vec3d right = dir.cross(openvdb::Vec3d(0, 1, 0));
    if (right.lengthSqr() < 1e-12) right = openvdb::Vec3d(1, 0, 0);
    right.normalize();
    const openvdb::Vec3d up = right.cross(dir).unit();
    const double scale = 2.0 * radius / std::max(w, h);  // world units per pixel
    const int steps = (int)std::ceil(2.0 * radius / step);
    std::vector<unsigned char> rgba((size_t)w * h * 4);
    tbb::parallel_for(tbb::blocked_range<int>(0, h), [&](const tbb::blocked_range<int>& r) {
        openvdb::tools::GridSampler<openvdb::FloatGrid, openvdb::tools::BoxSampler> sampler(*grid);
        for (int y = r.begin(); y != r.end(); ++y) {
            for (int x = 0; x < w; ++x) {
                const double px = (x + 0.5 - 0.5 * w) * scale, py = (0.5 * h - y - 0.5) * scale;
                const openvdb::Vec3d origin = centre + right * px + up * py - dir * radius;
                double tau = 0.0;
                for (int i = 0; i < steps; ++i) {
                    const openvdb::Vec3d p = origin + dir * ((i + 0.5) * step);
                    const float d = sampler.wsSample(p);
                    if (d > 0.0f) tau += (double)d * step;
                }
                const double a = 1.0 - std::exp(-tau);
                unsigned char* px4 = &rgba[((size_t)y * w + x) * 4];
                px4[0] = px4[1] = px4[2] = 235;
                px4[3] = (unsigned char)std::lround(std::min(1.0, std::max(0.0, a)) * 255.0);
            }
        }
    });
    writePng(outPath, w, h, rgba);
    std::ostringstream o;
    o << "{\"ok\":true,\"width\":" << w << ",\"height\":" << h << ",\"quality\":\"" << q->name << "\",\"steps\":" << steps
      << ",\"grid\":\"" << esc(grid->getName()) << "\",\"peakRssKb\":" << peakRssKb() << "}";
    return o.str();
}


// ---------- tiles: the bridge to the CFX interpreter (X2) ----------
// A tile file is little-endian: a 48 byte header (magic "CFXT", version 1, channels 1|3, leaf count, voxel size,
// background value of up to 3 channels), then per leaf: origin (3 x i32), active mask (8 x u64), 512 x channels
// f32 values in OpenVDB leaf order (x slowest). Active tiles are made into leaves first, so the host only ever
// sees leaves. The Rust side (crates/vdb/src/tiles.rs) reads and writes the same layout and checks every length.
static const size_t LEAF_VOXELS = 512;
static const size_t TILE_HEADER = 48;

template <class T> static void putv(std::vector<unsigned char>& v, T x) {
    unsigned char b[sizeof(T)];
    std::memcpy(b, &x, sizeof(T));  // the host is little-endian (Linux amd64)
    v.insert(v.end(), b, b + sizeof(T));
}
template <class T> static T getv(const std::vector<unsigned char>& v, size_t& at) {
    T x;
    if (at + sizeof(T) > v.size()) throw std::runtime_error("truncated tile file");
    std::memcpy(&x, v.data() + at, sizeof(T));
    at += sizeof(T);
    return x;
}

static openvdb::GridBase::Ptr readOneGrid(const std::string& path, const std::string& name) {
    openvdb::io::File file(path);
    file.setCopyMaxBytes(0);
    file.open();
    if (!file.hasGrid(name)) throw std::runtime_error("no grid named '" + name + "'");
    openvdb::GridBase::Ptr g = file.readGrid(name);
    file.close();
    return g;
}

static float comp(float v, int) { return v; }
static float comp(const openvdb::Vec3f& v, int c) { return v[c]; }
static float makeVal(float a, float, float, float*) { return a; }
static openvdb::Vec3f makeVal(float a, float b, float c, openvdb::Vec3f*) { return openvdb::Vec3f(a, b, c); }

template <class GridT> static uint64_t writeTiles(GridT& grid, int channels, FILE* f) {
    using ValueT = typename GridT::ValueType;
    grid.tree().voxelizeActiveTiles();
    std::vector<const typename GridT::TreeType::LeafNodeType*> leaves;
    for (auto it = grid.tree().cbeginLeaf(); it; ++it) leaves.push_back(&*it);
    std::vector<unsigned char> head;
    head.insert(head.end(), {'C', 'F', 'X', 'T'});
    putv<uint32_t>(head, 1);
    putv<uint32_t>(head, (uint32_t)channels);
    putv<uint32_t>(head, 0);
    putv<uint64_t>(head, (uint64_t)leaves.size());
    putv<double>(head, grid.voxelSize()[0]);
    const ValueT bg = grid.background();
    for (int c = 0; c < 3; ++c) putv<float>(head, channels == 1 ? (c == 0 ? comp(bg, 0) : 0.0f) : comp(bg, c));
    putv<uint32_t>(head, 0);
    if (std::fwrite(head.data(), 1, head.size(), f) != head.size()) throw std::runtime_error("short write");
    std::vector<unsigned char> buf;
    for (const auto* leaf : leaves) {
        buf.clear();
        const openvdb::Coord o = leaf->origin();
        putv<int32_t>(buf, o.x());
        putv<int32_t>(buf, o.y());
        putv<int32_t>(buf, o.z());
        const auto& m = leaf->valueMask();
        for (int w = 0; w < 8; ++w) putv<uint64_t>(buf, m.template getWord<uint64_t>(w));
        for (size_t i = 0; i < LEAF_VOXELS; ++i) {
            const ValueT v = leaf->getValue((openvdb::Index)i);
            for (int c = 0; c < channels; ++c) putv<float>(buf, comp(v, c));
        }
        if (std::fwrite(buf.data(), 1, buf.size(), f) != buf.size()) throw std::runtime_error("short write");
    }
    return leaves.size();
}

static std::string exportTiles(const std::string& in, const std::string& gridName, const std::string& outPath) {
    openvdb::GridBase::Ptr g = readOneGrid(in, gridName);
    FILE* f = std::fopen(outPath.c_str(), "wb");
    if (!f) throw std::runtime_error("cannot write " + outPath);
    uint64_t n = 0;
    int channels = 0;
    try {
        if (g->isType<openvdb::FloatGrid>()) { channels = 1; n = writeTiles(*openvdb::gridPtrCast<openvdb::FloatGrid>(g), 1, f); }
        else if (g->isType<openvdb::Vec3SGrid>()) { channels = 3; n = writeTiles(*openvdb::gridPtrCast<openvdb::Vec3SGrid>(g), 3, f); }
        else throw std::runtime_error("grid '" + gridName + "' has type " + g->valueType() + "; only float and vec3s grids are supported");
    } catch (...) { std::fclose(f); throw; }
    if (std::fclose(f) != 0) throw std::runtime_error("short write " + outPath);
    std::ostringstream o;
    o << "{\"ok\":true,\"leaves\":" << n << ",\"channels\":" << channels << ",\"voxelSize\":" << g->voxelSize()[0]
      << ",\"activeVoxels\":" << g->activeVoxelCount() << ",\"peakRssKb\":" << peakRssKb() << "}";
    return o.str();
}

static std::vector<unsigned char> slurp(const std::string& path) {
    std::ifstream f(path, std::ios::binary);
    if (!f) throw std::runtime_error("cannot read " + path);
    return std::vector<unsigned char>((std::istreambuf_iterator<char>(f)), std::istreambuf_iterator<char>());
}

template <class GridT> static typename GridT::Ptr gridFromTiles(const std::vector<unsigned char>& d, int channels, const openvdb::math::Transform::Ptr& xf, const std::string& name, uint64_t nleaves, size_t at, const float* bg) {
    using ValueT = typename GridT::ValueType;
    const ValueT background = makeVal(bg[0], bg[1], bg[2], (ValueT*)nullptr);
    typename GridT::Ptr grid = GridT::create(background);
    grid->setTransform(xf->copy());
    grid->setName(name);
    for (uint64_t l = 0; l < nleaves; ++l) {
        const int32_t x = getv<int32_t>(d, at), y = getv<int32_t>(d, at), z = getv<int32_t>(d, at);
        if ((x & 7) || (y & 7) || (z & 7)) throw std::runtime_error("tile origin is not a leaf origin");
        uint64_t words[8];
        for (int w = 0; w < 8; ++w) words[w] = getv<uint64_t>(d, at);
        auto* leaf = new typename GridT::TreeType::LeafNodeType(openvdb::Coord(x, y, z), background, false);
        for (size_t i = 0; i < LEAF_VOXELS; ++i) {
            float a = getv<float>(d, at), b = 0, c = 0;
            if (channels == 3) { b = getv<float>(d, at); c = getv<float>(d, at); }
            const ValueT v = makeVal(a, b, c, (ValueT*)nullptr);
            if (words[i >> 6] & (uint64_t(1) << (i & 63))) leaf->setValueOn((openvdb::Index)i, v);
            else leaf->setValueOnly((openvdb::Index)i, v);
        }
        grid->tree().addLeaf(leaf);
    }
    return grid;
}

static std::string importTiles(const std::string& ref, const std::string& refGrid, const std::string& outPath, const std::vector<std::pair<std::string, std::string>>& tiles) {
    openvdb::io::File in(ref);
    in.setCopyMaxBytes(0);
    in.open();
    openvdb::GridPtrVecPtr grids = in.getGrids();
    openvdb::MetaMap::Ptr meta = in.getMetadata();
    in.close();
    openvdb::math::Transform::Ptr xf;
    for (const auto& g : *grids) if (g->getName() == refGrid) xf = g->transformPtr();
    if (!xf) throw std::runtime_error("no reference grid named '" + refGrid + "'");
    // a new grid replaces an existing one of the same name
    for (const auto& t : tiles)
        for (auto it = grids->begin(); it != grids->end();)
            if ((*it)->getName() == t.first) it = grids->erase(it); else ++it;
    uint64_t total = 0;
    for (const auto& t : tiles) {
        const std::vector<unsigned char> d = slurp(t.second);
        size_t at = 0;
        if (d.size() < TILE_HEADER || std::memcmp(d.data(), "CFXT", 4) != 0) throw std::runtime_error("not a tile file: " + t.second);
        at = 4;
        if (getv<uint32_t>(d, at) != 1) throw std::runtime_error("unsupported tile file version");
        const uint32_t channels = getv<uint32_t>(d, at);
        getv<uint32_t>(d, at);
        const uint64_t n = getv<uint64_t>(d, at);
        getv<double>(d, at);
        float bg[3];
        for (int c = 0; c < 3; ++c) bg[c] = getv<float>(d, at);
        getv<uint32_t>(d, at);
        if (channels != 1 && channels != 3) throw std::runtime_error("bad channel count");
        const uint64_t per = 12 + 64 + (uint64_t)LEAF_VOXELS * channels * 4;
        if (n > (d.size() - TILE_HEADER) / per || d.size() - TILE_HEADER != n * per) throw std::runtime_error("tile file size does not match its header");
        total += n;
        if (channels == 1) grids->push_back(gridFromTiles<openvdb::FloatGrid>(d, 1, xf, t.first, n, at, bg));
        else grids->push_back(gridFromTiles<openvdb::Vec3SGrid>(d, 3, xf, t.first, n, at, bg));
    }
    openvdb::io::File out(outPath);
    out.write(*grids, *meta);
    out.close();
    std::ostringstream o;
    o << "{\"ok\":true,\"leaves\":" << total << ",\"peakRssKb\":" << peakRssKb() << "}";
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
        if ((argc == 8 || argc == 10) && !std::strcmp(argv[1], "render")) {
            // render <in.vdb> <grid|-> <out.png> <width> <height> <quality> [azimuth elevation]
            const double az = argc == 10 ? std::atof(argv[8]) : 30.0, el = argc == 10 ? std::atof(argv[9]) : 20.0;
            std::cout << render(argv[2], std::strcmp(argv[3], "-") ? argv[3] : "", argv[4], std::atoi(argv[5]), std::atoi(argv[6]), argv[7], az, el) << "\n";
            return 0;
        }
        if (argc == 5 && !std::strcmp(argv[1], "export-tiles")) {
            std::cout << exportTiles(argv[2], argv[3], argv[4]) << "\n";
            return 0;
        }
        if (argc >= 7 && (argc - 5) % 2 == 0 && !std::strcmp(argv[1], "import-tiles")) {
            std::vector<std::pair<std::string, std::string>> tl;
            for (int i = 5; i + 1 < argc; i += 2) tl.emplace_back(argv[i], argv[i + 1]);
            std::cout << importTiles(argv[2], argv[3], argv[4], tl) << "\n";
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
