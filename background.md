
# Getting Started
Jump to navigationJump to search
First of all, developing an operating system is probably one of the most challenging things you can do on a computer (next to killing the final boss in Doom on Nightmare difficulty level). Composing an operating system requires a lot of knowledge about several complex areas within computer science. You need to understand how hardware works and be able to read and write the complex assembly language as well as a higher-level language (such as C, C++, or Pascal). Your mind has to be able to wrap itself around abstract theory and hold a myriad of thoughts. Feel discouraged yet? Don't fear! Because all of these things are also the things that makes OS programming fun and entertaining.

There is nothing like the feeling of accomplishment when you, finally, after hours of struggling, solve the problem. And after some time you are able to look back and see all of the things you've created from scratch. Your handwritten system is able to boot, performs magic against the hardware, and gives the user a user interface and programs to play with.

There is no absolute path you have to take when creating an OS. Once you get your initial system up and running (and you do this by finding appropriate tutorials), you choose the path you want to take next. Your OS is exactly that--yours. You have ultimate control, and the sky's the limit!


Contents
1	The Hard Truth
2	Responsibility
3	Required Knowledge
4	Organize your plans
5	Choosing your development environment
5.1	GNU/Linux
5.2	Windows
5.3	MacOS
6	Testing your operating system
7	Protecting your code
8	Common starting points
9	Obtaining further knowledge
10	See Also
10.1	Articles
10.2	Threads
10.3	External Links
The Hard Truth
Hopefully the basic fact that operating system development is a complicated and ongoing process does not discourage you. The truth is, operating system development is truly unparalleled since it requires the utmost amount of patience and careful code design, and it returns very little to no "instant gratification" you get from the development of things like games and web-based scripting.

You have been fairly warned of the hard work ahead, but if you are still interested then proceed forward into the realm of the operating system programmer. Prepare yourself for occasional bouts of confusion, discouragement, and for some of us...temporary insanity. In time, and with enough dedication, you will find yourself among the elite few who have contributed to a working operating system. If you do get discouraged along the way, refresh yourself with the content of this book. Hopefully it will remind you why you started such an insane journey in the first place.

At this stage, it would also pay to read the Beginner Mistakes page. Users on the forum have noticed a lot of these mistakes getting repeated over time, and avoiding them is a great way to not make a fool of yourself.

Responsibility
People tend to claim that it is OK to write inefficient software, stating that computer systems are so fast these days, that you won't see the impact. This type of mentality is dangerous in operating system design. It might be OK to write sloppy code when making a simple application, but when it comes to critical code that may get called thousands of times per second, you need to take out all the overhead you can. The operating system should supply the computer as a basic resource to the running applications, with as little complication, abstraction, and overhead as possible.

People who design operating systems in this day and age tend to have the "everything but the kitchen sink" mentality. They take it upon themselves to account for everything, which of course is good, but it shouldn't be done at the expense of allowing poorly-written programs to flourish. There are many things that go on "under the hood" when program errors occur. Poorly-written programs cost precious execution time and involve task switches that are expensive in both memory and frequency. We encourage you to discourage poorly-written software.

Required Knowledge
Main article: Required Knowledge
If you think you can skip this, it's just for you.

This section has been moved to a separate page because it is referred to so often in forum discussions.

Organize your plans
Before proceeding, consider what it is you want to get out of writing an operating system. What are your motivations in taking on this project? There are many possible reasons for taking on a hobby OS project, and most os-devers have more than one. Even just saying, "I just want to" can be enough, though the more you consider and clarify your goals and motives, the more you can focus on what you really want.

Be honest with yourself, as well. There's no shame in having larger ambitions for your project, even (or especially) if they aren't the primary objective. Try to acknowledge all of your goals, not just the one you think is your main purpose.

Try to settle on which aspects of OS design you are most interested in or see a need to work on. Most of what goes into OS dev, especially early on, is kernel design and development, but the kernel itself is only a small part of most operating systems; if your primary interest is in UX, or networking, or driver programming, you should think about whether you really need (now or in the future) to write your own OS at all or if you would be just as satisfied developing those things on an existing kernel. More than a few people have gone into OS dev when they really wanted to design a desktop environment, so this is a very important question to ask yourself.

Try to think of any non-OS projects you might want to take on first, or at the same time, especially ones which might serve as practice or preparation for the OS project. There's usually no need to work on the OS project right now, and the more you have prepared ahead of time, the better off you'll be (up to a point, at least--preparation is one thing, procrastination is something else).

Similarly, if you mean to work on forking an existing design to experiment with, or to modify for some specific purpose, focus on that rather than general development issues. Consider what part of the existing code base you will need, and which ones you want to change.

Try to work out some of your specific project goals, and be prepared to plan out separate projects if it helps to do so. If you are simply intending to putter around and see where it takes you, that's fine; if your intent is to overthrow Microsoft, that's fine (if probably unrealistic), too. Once you know what you want to do, you can break down the details of it into specific objectives, and work out what it would take to reach them. Don't try to force too many divergent objectives into one project--if you have different things you want to try with contradictory goals, split them into different projects.

It may help if you write out an overview of your planned OS design, with any specific requirements or details you feel are notable or which could clarify what you need help with, and add it to your public repository if you can. This not only will make it easier for others to help you, it will help organize and stabilize your plans, much like writing an outline for a story or paper. Be prepared to maintain it as your goals and plans change, but keep a copy of older versions (or better still, keep the document under version control) so you can see how your work develops over time.

Finally, review the time and resources which the project will require, and decide if they are feasible. If you know that you only have a certain amount of time to devote to the project, take that into account, and whatever you do, don't commit to an outside deadline even if you are certain you can reach it. OS dev takes time--a lot of time--and trying to finish a full OS project in a semester isn't realistic.

Choosing your development environment
You need a platform to develop your new system on. Following the trends of general computing, the most popular is GNU/Linux, but many use Windows too. Developers using a GNU/Linux system have a slight advantage in availability of tools, but this can be solved on Windows using a system such as Cygwin or MinGW.

Binutils: Fundamental tools for manipulation of object files.
GCC: The GNU Compiler Collection. GCC contains compilers for C, C++, Fortran, and Ada, amongst others.
Make: For automating the build process, which becomes really helpful once you have more than a handful of files.
Grep and sed: For doing more powerful searches and search-and-replaces (helpful when filling out tables with data).
Diffutils: Incredibly useful for showing the differences between two files.
Perl or Python: One of these two scripting languages should be installed. Useful for string manipulation, amongst other things. Perl used to be the recommendation, but Python is now quite mature and is possibly easier to learn. Both have hundreds of packages/modules available for doing various tasks.
An Assembler: For example NASM or GAS. This varies depending on your target CPU architecture.
An editor: For writing your Assembly, C, and other (code) files.
You might not use all of these tools, but it is best to have them on hand "just in case," and know how to use them, even at a basic level. But if you decided to use another language then the tooling is mostly up to you and maybe the list above just won't help you in any way. Below is the information mostly related to the C/C++ or Assembly developers.

GNU/Linux
The most recommended system for OS development is GNU/Linux. When using GNU/Linux, most of the GNU development tools are probably already present. If not, use your distribution's package management tools (APT, RPM, Portage, Pacman, Apk, etc.) to install them as needed. Again, making a cross-compiler is required, so as not to link in the development system's runtime files.

Common editors are Vim, Emacs, KDevelop, Komodo Edit, etc. Some prefer lightweight editors instead of an IDE, such as gedit, Geany and SciTE. Many like Midnight Commander which has a Text UI and a built-in editor (mcedit) and therefore extremely lightweight and lightning fast.

About which distributions you should use, consult the list of Linux distributions. They come in all shapes and sizes, but as long as they're relatively general-purpose, they should be fine.

If you are unsure, try Ubuntu, Fedora or Linux Mint.

Windows
In order to get the tools necessary, you should install the Cygwin environment. MinGW or DJGPP are alternatives, but MSYS2 is strongly suggested as it is the most complete and compatible environment, and also includes a package manager to install libraries and tools.

Microsoft has recently (as of writing) released the Windows Subsystem for Linux as an optional feature for Windows 10. It is basically a real Ubuntu command line distribution running on top of Windows WITHOUT the use of a VM. The latest GCC and Binutils (6.1.0 and 2.27 as of writing) compile and work correctly in this environment. Using the Bash shell, you can access your Windows hard disks through /mnt/<drive letter>. The advantage of this solution is that you can work with whichever Windows or Linux tools that you require, without having to find out if they work in Cygwin. Many of the needed tools can be installed using "apt-get".

For all of the above, it is strongly suggested to build a cross-compiler, not only because the default compilers target different executable formats, but because it's generally a good idea. Check the GCC Cross-Compiler page for details and instructions.

You will also need an editor. Using Notepad will work, but it's easier if you have a more complete editor. For example, Notepad++ or Notepad2 are used by many people. If you are comfortable with Unix editors, you can pick one from the choice Cygwin provides (which includes e.g. Vim and Emacs, which take some getting-used-to but are very powerful).

It is also possible to use Visual Studio, or the freely downloadable Visual C++ Express Edition, to write and compile your operating system. You will require a special configuration file, and you will certainly be in the minority, but it does work quite well. You can even install the Windows SDK on top, enabling 64 bit development. The only pitfall is this doesn't support Inline Assembly.

Other tools such as Watcom or Borland can be used, too, but they each have specific requirements of their own, and are not widely used for this kind of work.

Another consideration is that you will probably have as a goal for your OS to be self-hosting, that is, you can compile your operating system using your operating system. If your OS is written in C, your minimal requirements will therefore be a C compiler and C library. If you intend for your OS to be a Windows clone rather than another POSIX-compliant OS, you will need a C library that does Windows calls instead of POSIX calls, and you will need a C compiler that uses just the C library instead of doing POSIX calls. GCCWIN + PDPCLIB fits this bill.

MacOS
Because under the hood it uses FreeBSD's userland, it is fully POSIX compatible. All the usual tools are available (vi, bash, dd, cat, sed, tar, cpio, etc.) Almost every tutorial works out-of-the-box. The missing tools are mostly file system related: no loopback device, no fdisk, no mkfs.vfat nor mtools. But you can use diskutil for these purposes, or use brew or macports to install those missing tools.

To get gcc, you used to have an mpkg on the 2nd Installation DVD for the older versions. Newer MacOS versions (10.13 and up) can install command line XCode (not the IDE, just the toolchain) by running "xcode-select --install" from a Terminal. This will install gcc, binutils and make. This gcc is actually a masquaraded CLang, but featurefull enough to build your own cross-compiler without problems. It is preferred to use the official compiler for bootstraping gcc than to install one from brew or macports.

Testing your operating system
Main article: Testing
The above article goes into a lot of depth about choosing how to test your operating system and how to integrate that with your development process. Both physical and emulated testing environments are discussed.

Protecting your code
During your code building you will write hundreds, even thousands, of lines of code. You'll spend an unmentionable number of hours, and sit up late at night coding when you really should go to bed. The last thing you need is a disk crash or a poorly written 'rm' or 'format' command throwing all your work away.

What you need is a version control system. CVS has been used for a number of years, but has gotten a lot of competition from Subversion, Bazaar, Mercurial, and Git lately. If you can, you should set up a remote computer or server as a version control server, but if you do not have such a machine available you can also host the version control system on your local development computer. Just remember to backup your code to CD or FTP once in a while.

We cannot stress this point strongly enough: if you are not using source control already, you should start doing so immediately. You only need make a serious mistake in your code once to realize the importance of having your code securely versioned and easily retrievable. While it may seem like overkill for a small, private hobby project, once you get into the habit of using revision control, you'll wonder how you ever did without it.

For Git you can create your project on GitHub or Bitbucket. Both come with free, private repositories.

An additional benefit of using version control on a network-accessible repository is that it makes it a lot easier to collaborate with and get help from others. This can be quite useful, especially in the forums, as it can avoid the need for constantly posting updated versions of your code to a message thread--you simply point the conversation towards your repository, and the others in the thread will have direct access to your most current changes. It is also crucial if, as the project grows, you begin to work with other developers on the project (just don't expect that to happen overnight).

Common starting points
The easiest way to get a "Hello World" 64-bit higher half kernel going is the Limine Bare Bones tutorial. A different approach would be to learn how the computer itself starts up, on the Boot Sequence page.

# Hello World


Limine Bare Bones
Jump to navigationJump to search

The Limine Boot Protocol is the native boot protocol provided by the Limine bootloader. It is designed to overcome shortcomings of common boot protocols used by hobbyist OS developers, such as Multiboot.

It provides cutting edge features such as 5-level paging support, 64-bit Long Mode support, and direct higher half kernel loading.

The Limine boot protocol is firmware and architecture agnostic. It supports x86-64, aarch64, riscv64, and loongarch64.

This article will demonstrate how to write a small Limine-compliant x86-64 kernel in (GNU) C, and boot it using the Limine bootloader.

Additionally, it is highly recommended to check out this repository as it provides more complete, buildable, portable template code to go along with this guide.


Contents
1	Overview
1.1	src/main.c
1.2	linker.lds
2	Building the kernel and creating an image
2.1	GNUmakefile
2.2	limine.conf
2.3	Compiling the kernel
2.4	Compiling the kernel on macOS
2.5	Creating the image
2.5.1	Creating an ISO
2.5.2	Creating a hard disk/USB drive image
3	Conclusions
4	See Also
4.1	Articles
4.2	External Links
Overview
For this example, we will create these 2 files to create the basic directory tree of our project:

src/main.c
linker.lds
As one may notice, there is no "entry point" assembly stub, as one is not necessary with the Limine protocol when using a language which can make use of a standard SysV x86 calling convention.

Furthermore, we will download the header file limine.h which defines structures and constants that we will use to interact with the bootloader from here, and place it in the src directory.

Obviously, this is just a bare bones example, and one should always refer to the Limine protocol specification for more details and information.

src/main.c
This is the kernel "main".

#include <stdint.h>
#include <stddef.h>
#include <stdbool.h>
#include <limine.h>

// Set the base revision to 6, this is recommended as this is the latest
// base revision described by the Limine boot protocol specification.
// See specification for further info.

__attribute__((used, section(".limine_requests")))
static volatile uint64_t limine_base_revision[] = LIMINE_BASE_REVISION(6);

// The Limine requests can be placed anywhere, but it is important that
// the compiler does not optimise them away, so, usually, they should
// be made volatile or equivalent, _and_ they should be accessed at least
// once or marked as used with the "used" attribute as done here.

__attribute__((used, section(".limine_requests")))
static volatile struct limine_framebuffer_request framebuffer_request = {
    .id = LIMINE_FRAMEBUFFER_REQUEST_ID,
    .revision = 0
};

// Finally, define the start and end markers for the Limine requests.
// These can also be moved anywhere, to any .c file, as seen fit.

__attribute__((used, section(".limine_requests_start")))
static volatile uint64_t limine_requests_start_marker[] = LIMINE_REQUESTS_START_MARKER;

__attribute__((used, section(".limine_requests_end")))
static volatile uint64_t limine_requests_end_marker[] = LIMINE_REQUESTS_END_MARKER;

// GCC and Clang reserve the right to generate calls to the following
// 4 functions even if they are not directly called.
// Implement them as the C specification mandates.
// DO NOT remove or rename these functions, or stuff will eventually break!
// They CAN be moved to a different .c file.

void *memcpy(void *restrict dest, const void *restrict src, size_t n) {
    uint8_t *restrict pdest = dest;
    const uint8_t *restrict psrc = src;

    for (size_t i = 0; i < n; i++) {
        pdest[i] = psrc[i];
    }

    return dest;
}

void *memset(void *s, int c, size_t n) {
    uint8_t *p = s;

    for (size_t i = 0; i < n; i++) {
        p[i] = (uint8_t)c;
    }

    return s;
}

void *memmove(void *dest, const void *src, size_t n) {
    uint8_t *pdest = dest;
    const uint8_t *psrc = src;

    if ((uintptr_t)src > (uintptr_t)dest) {
        for (size_t i = 0; i < n; i++) {
            pdest[i] = psrc[i];
        }
    } else if ((uintptr_t)src < (uintptr_t)dest) {
        for (size_t i = n; i > 0; i--) {
            pdest[i-1] = psrc[i-1];
        }
    }

    return dest;
}

int memcmp(const void *s1, const void *s2, size_t n) {
    const uint8_t *p1 = s1;
    const uint8_t *p2 = s2;

    for (size_t i = 0; i < n; i++) {
        if (p1[i] != p2[i]) {
            return p1[i] < p2[i] ? -1 : 1;
        }
    }

    return 0;
}

// Halt and catch fire function.
static void hcf(void) {
    for (;;) {
        asm ("hlt");
    }
}

// The following will be our kernel's entry point.
// If renaming kmain() to something else, make sure to change the
// linker script accordingly.
void kmain(void) {
    // Ensure the bootloader actually understands our base revision (see spec).
    if (LIMINE_BASE_REVISION_SUPPORTED(limine_base_revision) == false) {
        hcf();
    }

    // Ensure we got a framebuffer.
    if (framebuffer_request.response == NULL
     || framebuffer_request.response->framebuffer_count < 1) {
        hcf();
    }

    // Fetch the first framebuffer.
    struct limine_framebuffer *framebuffer = framebuffer_request.response->framebuffers[0];

    // Print a nice pattern to screen as an example.
    // Note: we assume the framebuffer model is RGB with 32-bit pixels.
    volatile uint32_t *fb_ptr = framebuffer->address;
    for (size_t y = 0; y < framebuffer->height; y++) {
        for (size_t x = 0; x < framebuffer->width; x++) {
            uint32_t nX = x * 255 / framebuffer->width;
            uint32_t nY = y * 255 / framebuffer->height;
            fb_ptr[y * (framebuffer->pitch / 4) + x] = (nY << 8) | nX;
        }
    }

    // We're done, just hang...
    hcf();
}
linker.lds
This is going to be our linker script describing where our sections will end up in memory.

/* Tell the linker that we want an x86_64 ELF64 output file */
OUTPUT_FORMAT(elf64-x86-64)

/* We want the symbol kmain to be our entry point */
ENTRY(kmain)

/* Define the program headers we want so the bootloader gives us the right */
/* MMU permissions; this also allows us to exert more control over the linking */
/* process. */
PHDRS
{
    limine_requests PT_LOAD;
    text PT_LOAD;
    rodata PT_LOAD;
    data PT_LOAD;
}

SECTIONS
{
    /* We want to be placed in the topmost 2GiB of the address space, for optimisations */
    /* and because that is what the Limine spec mandates. */
    /* Any address in this region will do, but often 0xffffffff80000000 is chosen as */
    /* that is the beginning of the region. */
    . = 0xffffffff80000000;

    /* Define a section to contain the Limine requests and assign it to its own PHDR */
    .limine_requests : {
        KEEP(*(.limine_requests_start))
        KEEP(*(.limine_requests))
        KEEP(*(.limine_requests_end))
    } :limine_requests

    /* Move to the next memory page for .text */
    . = ALIGN(CONSTANT(MAXPAGESIZE));

    .text : {
        *(.text .text.*)
    } :text

    /* Move to the next memory page for .rodata */
    . = ALIGN(CONSTANT(MAXPAGESIZE));

    .rodata : {
        *(.rodata .rodata.*)
    } :rodata

    /* Add a .note.gnu.build-id output section in case a build ID flag is added to the */
    /* linker command. */
    .note.gnu.build-id : {
        *(.note.gnu.build-id)
    } :rodata

    /* Move to the next memory page for .data */
    . = ALIGN(CONSTANT(MAXPAGESIZE));

    .data : {
        *(.data .data.*)
    } :data

    /* NOTE: .bss needs to be the last thing mapped to :data, otherwise lots of */
    /* unnecessary zeros will be written to the binary. */
    /* If you need, for example, .init_array and .fini_array, those should be placed */
    /* above this. */
    .bss : {
        *(.bss .bss.*)
        *(COMMON)
    } :data

    /* Discard .note.* and .eh_frame* since they may cause issues on some hosts. */
    /DISCARD/ : {
        *(.eh_frame*)
        *(.note .note.*)
    }
}
Building the kernel and creating an image
GNUmakefile
In order to build our kernel, we are going to use a Makefile. Since we're going to use GNU make specific features, we call this file GNUmakefile instead, so only GNU make will process it.

# Nuke built-in rules.
.SUFFIXES:

# This is the name that our final executable will have.
# Change as needed.
override OUTPUT := myos

# User controllable toolchain and toolchain prefix.
TOOLCHAIN :=
TOOLCHAIN_PREFIX :=
ifneq ($(TOOLCHAIN),)
    ifeq ($(TOOLCHAIN_PREFIX),)
        TOOLCHAIN_PREFIX := $(TOOLCHAIN)-
    endif
endif

# User controllable C compiler command.
ifneq ($(TOOLCHAIN_PREFIX),)
    CC := $(TOOLCHAIN_PREFIX)gcc
else
    CC := cc
endif

# User controllable linker command.
LD := $(TOOLCHAIN_PREFIX)ld

# Defaults overrides for variables if using "llvm" as toolchain.
ifeq ($(TOOLCHAIN),llvm)
    CC := clang
    LD := ld.lld
endif

# User controllable C flags.
CFLAGS := -g -O2 -pipe

# User controllable C preprocessor flags. We set none by default.
CPPFLAGS :=

# User controllable nasm flags.
NASMFLAGS := -g

# User controllable linker flags. We set none by default.
LDFLAGS :=

# Check if CC is Clang.
override CC_IS_CLANG := $(shell ! $(CC) --version 2>/dev/null | grep -q '^Target: '; echo $$?)

# If the C compiler is Clang, set the target as needed.
ifeq ($(CC_IS_CLANG),1)
    override CC += \
        -target x86_64-unknown-none-elf
endif

# Internal C flags that should not be changed by the user.
override CFLAGS += \
    -Wall \
    -Wextra \
    -std=gnu11 \
    -ffreestanding \
    -fno-stack-protector \
    -fno-stack-check \
    -fno-lto \
    -fno-PIC \
    -ffunction-sections \
    -fdata-sections \
    -m64 \
    -march=x86-64 \
    -mabi=sysv \
    -mno-80387 \
    -mno-mmx \
    -mno-sse \
    -mno-sse2 \
    -mno-red-zone \
    -mcmodel=kernel

# Internal C preprocessor flags that should not be changed by the user.
override CPPFLAGS := \
    -I src \
    $(CPPFLAGS) \
    -MMD \
    -MP

# Internal nasm flags that should not be changed by the user.
override NASMFLAGS := \
    -f elf64 \
    $(patsubst -g,-g -F dwarf,$(NASMFLAGS)) \
    -Wall

# Internal linker flags that should not be changed by the user.
override LDFLAGS += \
    -m elf_x86_64 \
    -nostdlib \
    -static \
    -z max-page-size=0x1000 \
    --gc-sections \
    -T linker.lds

# Use "find" to glob all *.c, *.S, and *.asm files in the tree and obtain the
# object and header dependency file names.
override SRCFILES := $(shell find -L src -type f 2>/dev/null | LC_ALL=C sort)
override CFILES := $(filter %.c,$(SRCFILES))
override ASFILES := $(filter %.S,$(SRCFILES))
override NASMFILES := $(filter %.asm,$(SRCFILES))
override OBJ := $(addprefix obj/,$(CFILES:.c=.c.o) $(ASFILES:.S=.S.o) $(NASMFILES:.asm=.asm.o))
override HEADER_DEPS := $(addprefix obj/,$(CFILES:.c=.c.d) $(ASFILES:.S=.S.d))

# Default target. This must come first, before header dependencies.
.PHONY: all
all: bin/$(OUTPUT)

# Include header dependencies.
-include $(HEADER_DEPS)

# Link rules for the final executable.
bin/$(OUTPUT): GNUmakefile linker.lds $(OBJ)
	mkdir -p "$(dir $@)"
	$(LD) $(LDFLAGS) $(OBJ) -o $@

# Compilation rules for *.c files.
obj/%.c.o: %.c GNUmakefile
	mkdir -p "$(dir $@)"
	$(CC) $(CFLAGS) $(CPPFLAGS) -c $< -o $@

# Compilation rules for *.S files.
obj/%.S.o: %.S GNUmakefile
	mkdir -p "$(dir $@)"
	$(CC) $(CFLAGS) $(CPPFLAGS) -c $< -o $@

# Compilation rules for *.asm (nasm) files.
obj/%.asm.o: %.asm GNUmakefile
	mkdir -p "$(dir $@)"
	nasm $(NASMFLAGS) $< -o $@

# Remove object files and the final executable.
.PHONY: clean
clean:
	rm -rf bin obj
limine.conf
This file is parsed by Limine and it describes boot entries and other bootloader configuration variables. Further information here.

# Timeout in seconds that Limine will use before automatically booting.
timeout: 5

# The entry name that will be displayed in the boot menu.
/myOS
    # We use the Limine boot protocol.
    protocol: limine

    # Path to the kernel to boot. boot():/ represents the partition on which limine.conf is located.
    path: boot():/boot/myos
Compiling the kernel
We can now build our example kernel by running make. This command, if successful, should generate, inside the bin directory, a file called myos (or the chosen kernel name). This is our Limine protocol-compliant kernel executable.

Compiling the kernel on macOS
If you are not using macOS, you can skip this section.

The macOS Xcode toolchain uses Mach-O binaries, and not the ELF binaries required for this Limine-compliant kernel. A solution is to build a GCC Cross-Compiler, or to obtain one from homebrew by installing the x86_64-elf-gcc package. After one of these is done, build using make TOOLCHAIN_PREFIX=x86_64-elf-.

Creating the image
We can now create either an ISO or a hard disk/USB drive image with our kernel on it. Limine can boot on both BIOS and UEFI if the image is set up to do so, which is what we are going to do.

Creating an ISO
In this example we are going to create a CD-ROM ISO capable of booting on both UEFI and legacy BIOS systems.

For this to work, we will need the xorriso utility.

These are shell commands. They can also be compiled into a script or Makefile.

# Download the latest Limine binary release.
curl -L https://github.com/Limine-Bootloader/Limine/releases/latest/download/limine-binary.tar.gz | gunzip | tar -xf -

# Build "limine" utility.
make -C limine-binary

# Create a directory which will be our ISO root.
mkdir -p iso_root

# Copy the relevant files over.
mkdir -p iso_root/boot
cp -v bin/myos iso_root/boot/
mkdir -p iso_root/boot/limine
cp -v limine.conf limine-binary/limine-bios.sys limine-binary/limine-bios-cd.bin \
      limine-binary/limine-uefi-cd.bin iso_root/boot/limine/

# Create the EFI boot tree and copy Limine's EFI executables over.
mkdir -p iso_root/EFI/BOOT
cp -v limine-binary/BOOTX64.EFI iso_root/EFI/BOOT/
cp -v limine-binary/BOOTIA32.EFI iso_root/EFI/BOOT/

# Create the bootable ISO.
xorriso -as mkisofs -R -r -J -b boot/limine/limine-bios-cd.bin \
        -no-emul-boot -boot-load-size 4 -boot-info-table -hfsplus \
        -apm-block-size 2048 --efi-boot boot/limine/limine-uefi-cd.bin \
        -efi-boot-part --efi-boot-image --protective-msdos-label \
        iso_root -o image.iso

# Install Limine stage 1 and 2 for legacy BIOS boot.
./limine-binary/limine bios-install image.iso
Creating a hard disk/USB drive image
In this example, we'll create an MBR partition table using sgdisk, containing a single FAT partition, also known as the ESP in EFI terminology, which will store our kernel, configs, and bootloader.

This example is more involved and is made up of more steps than creating an ISO image.

These are shell commands. They can also be compiled into a script or Makefile.

# Create an empty zeroed-out 64MiB image file.
dd if=/dev/zero bs=1M count=0 seek=64 of=image.hdd

# Create a partition table.
PATH=$PATH:/usr/sbin:/sbin sgdisk image.hdd -n 1:2048 -t 1:ef00 -m 1

# Download the latest Limine binary release.
curl -L https://github.com/Limine-Bootloader/Limine/releases/latest/download/limine-binary.tar.gz | gunzip | tar -xf -

# Build "limine" utility.
make -C limine-binary

# Install the Limine BIOS stages onto the image.
./limine-binary/limine bios-install image.hdd

# Format the image as fat32.
mformat -i image.hdd@@1M

# Make relevant subdirectories.
mmd -i image.hdd@@1M ::/EFI ::/EFI/BOOT ::/boot ::/boot/limine

# Copy over the relevant files.
mcopy -i image.hdd@@1M bin/myos ::/boot
mcopy -i image.hdd@@1M limine.conf limine-binary/limine-bios.sys ::/boot/limine
mcopy -i image.hdd@@1M limine-binary/BOOTX64.EFI ::/EFI/BOOT
mcopy -i image.hdd@@1M limine-binary/BOOTIA32.EFI ::/EFI/BOOT
Conclusions
If everything above has been completed successfully, you should now have a bootable ISO or hard drive/USB image containing your 64-bit higher half Limine protocol-compliant kernel and Limine to boot it. Once the kernel is successfully booted, you should see a line printed on screen from the top left corner.