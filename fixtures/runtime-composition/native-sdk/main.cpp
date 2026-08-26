#include "tiny.h"
#include <iostream>
int main() {
    std::cout << "tiny=" << tiny_answer() << '\n';
    return tiny_answer() == 42 ? 0 : 1;
}
