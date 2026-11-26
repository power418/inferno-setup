#pragma once
#include <stddef.h>
#include <stdint.h>

extern "C" {
void* malloc(size_t sz);
void free(void* p);
void* memcpy(void* dst, const void* src, size_t n);
void* memset(void* dst, int c, size_t n);
size_t strlen(const char* s);
int strcmp(const char* a, const char* b);
int usleep(unsigned usec);
int sched_yield(void);
int getpid(void);
}
