import os
from ament_index_python.packages import get_package_share_directory
from launch import LaunchDescription
from launch.actions import DeclareLaunchArgument, IncludeLaunchDescription
from launch.launch_description_sources import PythonLaunchDescriptionSource
from launch.substitutions import LaunchConfiguration
from launch_ros.actions import Node

# Sanitize Snap library contamination from LD_LIBRARY_PATH if running from Snap VS Code / terminal
if 'LD_LIBRARY_PATH' in os.environ:
    os.environ['LD_LIBRARY_PATH'] = ':'.join([
        p for p in os.environ['LD_LIBRARY_PATH'].split(':')
        if '/snap/' not in p
    ])

def generate_launch_description():
    pkg_mio_sim = get_package_share_directory('mio_sim')
    sim_launch_path = os.path.join(pkg_mio_sim, 'launch', 'sim.launch.py')
    default_rviz_config = os.path.join(pkg_mio_sim, 'config', 'sim.rviz')

    rviz_config_arg = DeclareLaunchArgument(
        'rviz_config',
        default_value=default_rviz_config,
        description='Path to RViz configuration file'
    )

    rviz_config = LaunchConfiguration('rviz_config')

    # Include Main Simulation Launch
    sim_launch = IncludeLaunchDescription(
        PythonLaunchDescriptionSource(sim_launch_path)
    )

    # Launch RViz2 with simulation time
    rviz_node = Node(
        package='rviz2',
        executable='rviz2',
        arguments=['-d', rviz_config],
        parameters=[{'use_sim_time': True}],
        output='screen'
    )

    return LaunchDescription([
        rviz_config_arg,
        sim_launch,
        rviz_node,
    ])
