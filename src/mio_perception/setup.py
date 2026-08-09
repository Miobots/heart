from setuptools import find_packages, setup

package_name = "mio_perception"

setup(
    name=package_name,
    version="0.0.0",
    packages=find_packages(exclude=["test"]),
    data_files=[
        ("share/ament_index/resource_index/packages", ["resource/" + package_name]),
        ("share/" + package_name, ["package.xml"]),
    ],
    install_requires=["setuptools"],
    zip_safe=True,
    maintainer="Jalal Ahmed",
    maintainer_email="rayyan1106adeel@gmail.com",
    description="Advisory object detection. Never touches velocity.",
    license="TODO",
    entry_points={"console_scripts": []},
)
