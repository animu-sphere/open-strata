//
// Copyright 2017 Pixar
//
// Licensed under the terms set forth in the LICENSE.txt file available at
// https://openusd.org/license.
//
#ifndef {{NAME}}_API_H
#define {{NAME}}_API_H

#include "pxr/base/arch/export.h"

#if defined(PXR_STATIC)
#   define {{NAME}}_API
#   define {{NAME}}_API_TEMPLATE_CLASS(...)
#   define {{NAME}}_API_TEMPLATE_STRUCT(...)
#   define {{NAME}}_LOCAL
#else
#   if defined({{NAME}}_EXPORTS)
#       define {{NAME}}_API ARCH_EXPORT
#       define {{NAME}}_API_TEMPLATE_CLASS(...) ARCH_EXPORT_TEMPLATE(class, __VA_ARGS__)
#       define {{NAME}}_API_TEMPLATE_STRUCT(...) ARCH_EXPORT_TEMPLATE(struct, __VA_ARGS__)
#   else
#       define {{NAME}}_API ARCH_IMPORT
#       define {{NAME}}_API_TEMPLATE_CLASS(...) ARCH_IMPORT_TEMPLATE(class, __VA_ARGS__)
#       define {{NAME}}_API_TEMPLATE_STRUCT(...) ARCH_IMPORT_TEMPLATE(struct, __VA_ARGS__)
#   endif
#   define {{NAME}}_LOCAL ARCH_HIDDEN
#endif

#endif
