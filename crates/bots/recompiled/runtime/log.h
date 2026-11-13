#pragma once
#include <cstdio>
// Retain upstream diagnostics; stderr is captured in the app's boot log.
#define LOGI(...) do { std::fprintf(stderr, __VA_ARGS__); std::fprintf(stderr, "\n"); } while (0)
