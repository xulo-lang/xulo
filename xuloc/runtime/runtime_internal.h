// Shared definitions for the stage 1 bootstrap runtime.
//
// `runtime.c` is a unity build: it includes this header once, then each
// `runtime_*.inc` module in dependency order. Keeping one translation unit is
// deliberate — the generated C is compiled as a single file and the bootstrap
// build passes only that file to the host compiler. Split a module out of this
// header only when it becomes a separately compiled unit.

#ifndef XULOC_RUNTIME_INTERNAL_H
#define XULOC_RUNTIME_INTERNAL_H

// `popen`/`pclose` (and `lstat`) are POSIX, but strict `-std=c11` hides them
// behind a feature-test macro. Ask for the 2008 POSIX surface explicitly.
#if !defined(_WIN32) && !defined(_POSIX_C_SOURCE)
#define _POSIX_C_SOURCE 200809L
#endif
#if defined(__APPLE__) && !defined(_DARWIN_C_SOURCE)
// Re-expose BSD additions (`st_mtimespec`, `_SC_NPROCESSORS_ONLN`) that a
// strict POSIX surface hides.
#define _DARWIN_C_SOURCE
#endif

#include <limits.h>
#include <math.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <errno.h>

#ifdef _WIN32
#include <direct.h>
#include <fcntl.h>
#include <io.h>
#include <process.h>
#include <sys/stat.h>
#include <sys/utime.h>
#include <windows.h>
#define XR_POPEN _popen
#define XR_PCLOSE _pclose
#define XR_GETPID _getpid
#else
#include <dirent.h>
#include <fcntl.h>
#include <pwd.h>
#include <sys/stat.h>
#include <sys/time.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>
#ifdef __APPLE__
#include <mach-o/dyld.h>
#endif
#define XR_POPEN popen
#define XR_PCLOSE pclose
#define XR_GETPID getpid
#endif

enum {
    XT_NIL = 0,
    XT_INT = 1,
    XT_FLOAT = 2,
    XT_BOOL = 3,
    XT_STR = 4,
    XT_LIST = 5,
    XT_OBJ = 6,
    XT_SB = 7,
    XT_SET = 8
};

typedef struct XVal XVal;
typedef struct XStr XStr;
typedef struct XList XList;
typedef struct XField XField;
typedef struct XObj XObj;
typedef struct XSb XSb;
typedef struct XSet XSet;

struct XVal {
    int tag;
    long long i;
    double f;
    void *p;
};

struct XStr {
    long long len;   // Unicode scalar values
    long long bytes; // UTF-8 bytes
    char *data;
};

struct XList {
    long long len;
    long long cap;
    XVal *items;
};

struct XField {
    const char *name;
    XVal value;
};

struct XObj {
    const char *type;
    long long nfields;
    XField *fields;
    const char *variant; // non-NULL for an enum value
    long long payload_len;
    XVal *payload;
    int debug; // `#[derive(Debug)]`: render as `Type(field: value, …)`
};

struct XSb {
    long long len;
    long long bytes;
    char *data;
};

struct XSet {
    long long len;
    long long cap;
    XVal *items;
};

typedef XVal (*XrMethodFn)(long long argc, XVal *argv);

// Defined with the value predicates; declared here because the intrinsic
// helpers below use it.
int xr_cond(XVal v);
// Defined with the renderer; declared here because map keys stringify values.
XVal xr_str_of(XVal v);
// Defined with the comparison operators; the set helpers use structural
// equality to keep elements unique.
int xr_eq(XVal a, XVal b);
// Defined with the method helpers; map operations stringify non-string keys.
static XVal xr_key_string(XVal key);

#endif
