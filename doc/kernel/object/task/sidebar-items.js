initSidebarItems({"enum":[["TaskBlock",""],["TaskCreationError",""],["TaskState",""]],"fn":[["decode_capabilities","Decode a capability stream (as found in a task's image) into a set of capabilities as they're represented in the kernel. For the format that's being decoded here, refer to the `(3.1) Userspace/Capabilities` section of the Book."]],"struct":[["KernelStackAllocator",""],["Task",""],["TaskStack","Represents the layout of a task's usermode or kernelmode stack. A slot is allocated (contiguous from `slot_bottom` to `top`), but only a portion of it may initially be mapped into backing memory (contiguous from `stack_bottom` to `top`). A stack can be grown by allocating more backing memory and moving `stack_bottom` down towards `slot_bottom`."]]});